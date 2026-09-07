import { useCallback, useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

/** How often a running app looks for a new release. kari is often left open
 *  for days, so the check on start is not enough on its own. */
const EVERY_MS = 6 * 60 * 60 * 1000;

/** Where the update got to. `ready` means a new kari is on disk and the running
 *  one is the old one until it restarts. */
export type UpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "downloading"; version: string; done: number; total: number | null }
  | { kind: "ready"; version: string }
  | { kind: "none" }
  | { kind: "failed"; why: string };

/** Whether this build can replace itself.
 *
 *  The updater plugin is built for the desktop only. On Android the phone gets
 *  its build from Obtainium, and a call here would fail on a plugin that is not
 *  there, so ask before calling rather than catching after. */
export function updatesSupported(): boolean {
  if (typeof window === "undefined") return false;
  if (!("__TAURI_INTERNALS__" in window)) return false;
  return !/Android|iPhone|iPad/i.test(navigator.userAgent);
}

/** Keeps this app on the newest release.
 *
 *  `enabled` installs what it finds; off, the check still runs so that Settings
 *  can offer the update, and nothing is written until the user asks. Either way
 *  the app keeps running the old code: replacing the bundle does not replace
 *  the process, so `ready` is an offer to restart, never a restart.
 *
 *  `null` means the settings have not arrived yet. Nothing happens then. The
 *  caller cannot pass `true` while it waits: that would install an update on a
 *  machine whose owner turned this off, in the seconds before the answer came
 *  back — which is the one thing this must never do.
 */
export function useUpdater(enabled: boolean | null, onReady: (version: string) => void) {
  const [state, setState] = useState<UpdateState>({ kind: "idle" });
  /** A second check while one runs would download the same bundle twice. */
  const busy = useRef(false);
  /** A new kari is already on disk. The running process is still the old
   *  version, so every later check finds the same release again — and would
   *  download and install it again. Once is enough; the rest waits for the
   *  restart. */
  const installed = useRef(false);
  /** Found but not installed, because auto-update is off. */
  const [offered, setOffered] = useState<Update | null>(null);
  /** The newest callback, so the interval below never restarts on a render. */
  const ready = useRef(onReady);
  useEffect(() => {
    ready.current = onReady;
  });

  const install = useCallback(async (u: Update) => {
    setState({ kind: "downloading", version: u.version, done: 0, total: null });
    let done = 0;
    let total: number | null = null;
    await u.downloadAndInstall((e) => {
      if (e.event === "Started") total = e.data.contentLength ?? null;
      else if (e.event === "Progress") done += e.data.chunkLength;
      else if (e.event === "Finished") done = total ?? done;
      setState({ kind: "downloading", version: u.version, done, total });
    });
    setOffered(null);
    installed.current = true;
    setState({ kind: "ready", version: u.version });
    ready.current(u.version);
  }, []);

  /** Look once. `auto` is the timer talking; a click passes false and gets to
   *  see "kari is up to date", which a background check must never say. */
  const run = useCallback(
    async (auto: boolean) => {
      if (enabled === null) return;
      if (!updatesSupported() || busy.current || installed.current) return;
      busy.current = true;
      try {
        if (!auto) setState({ kind: "checking" });
        const u = await check();
        if (!u) {
          setOffered(null);
          setState(auto ? { kind: "idle" } : { kind: "none" });
          return;
        }
        if (auto && !enabled) {
          // Found, and left alone. Settings shows it and the user decides.
          setOffered(u);
          setState({ kind: "idle" });
          return;
        }
        await install(u);
      } catch (e) {
        // A failed check is not worth a dialog: the network is down, or GitHub
        // is. It goes in the console, and Settings shows it if asked.
        console.warn("update check failed", e);
        setState({ kind: "failed", why: String(e) });
      } finally {
        busy.current = false;
      }
    },
    [enabled, install],
  );

  useEffect(() => {
    if (enabled === null || !updatesSupported()) return;
    run(true);
    const t = window.setInterval(() => run(true), EVERY_MS);
    return () => window.clearInterval(t);
  }, [enabled, run]);

  return {
    state,
    /** The release found while auto-update was off, waiting for a click. */
    offered,
    /** Look now, and report the answer either way. */
    checkNow: () => run(false),
    /** Install the release Settings is offering. */
    installOffered: () => (offered ? install(offered) : Promise.resolve()),
    /** Quit and come back as the version that was just written. */
    restart: () => relaunch(),
  };
}

/** Quit and start the version that was just written. */
export const restartApp = () => relaunch();

/** What `useUpdater` hands back. Settings takes the whole thing. */
export type Updater = ReturnType<typeof useUpdater>;
