import { useEffect, useRef, useState } from "react";

/** Closing the top layer with the back button.
 *
 * A phone has no Escape key. The system back button is how a person closes a
 * sheet, and an app that ignores it leaves the sheet open, or leaves the app
 * with the sheet still on screen. The page itself cannot read that button.
 *
 * What the page can read is its own history. The Android web view turns a back
 * press into one step back in the history of the page while there is a step to
 * take, and leaves the app when there is none. So every layer that opens — a
 * card sheet, a dialog, a tab away from the board — pushes one history entry,
 * and one step back closes the layer that pushed it. With no layer open the
 * history has nothing to go back to, and the press leaves the app, which is
 * what a phone user expects.
 *
 * The same code answers the back button of a browser and the back gesture of a
 * trackpad, so a card opened in any client closes the same way.
 */

/** The history this stack drives. A test hands it a fake one. */
export interface BackHost {
  /** Add one entry, so a back press has a step to take. */
  push: () => void;
  /** Take one step back, as the back button does. */
  back: () => void;
  /** Call `cb` on every step back. Returns the way to stop listening. */
  listen: (cb: () => void) => () => void;
}

export interface BackStack {
  /** Open a layer. The returned call closes it from the code instead. */
  push: (close: () => void) => () => void;
  /** How many layers are open. For tests. */
  depth: () => number;
}

interface Layer {
  close: () => void;
  /** A back press took this layer off. Its history entry is already gone. */
  popped: boolean;
}

export function createBackStack(host: BackHost): BackStack {
  const layers: Layer[] = [];
  /** Steps back this code asked for itself. Such a step is not a back press,
   *  so it must close nothing. */
  let ours = 0;
  /**
   * Entries this stack owes the history: a layer closed on its own, and its
   * entry has to go, or the next back press spends itself on a layer that is
   * not there.
   *
   * The debt is paid one tick later, and a layer that opens in the same tick
   * takes the entry over instead. This matters because a step back is asked
   * for and answered later, while an entry is added at once: a layer that
   * closes as another opens would otherwise add an entry and drop one in an
   * order the browser decides. React does exactly that when it mounts a
   * component twice to check it, and the sheet then took two presses to close.
   */
  let debt = 0;
  let unlisten: (() => void) | null = null;

  const onPop = () => {
    if (ours > 0) {
      ours--;
      return;
    }
    const top = layers.pop();
    if (!top) return;
    top.popped = true;
    top.close();
  };

  const settle = () => {
    while (debt > 0) {
      debt--;
      ours++;
      host.back();
    }
  };

  return {
    depth: () => layers.length,
    push(close) {
      const layer: Layer = { close, popped: false };
      layers.push(layer);
      unlisten ??= host.listen(onPop);
      // The entries carry no identity, so one owed to the history serves this
      // layer as well as a new one would.
      if (debt > 0) debt--;
      else host.push();
      return () => {
        const i = layers.indexOf(layer);
        // A back press closed this layer already, and took its entry with it.
        if (i < 0) return;
        layers.splice(i, 1);
        debt++;
        queueMicrotask(settle);
      };
    },
  };
}

/** The history of this page, as a `BackHost`.
 *
 * `pushState` keeps the address and adds one entry, so no layer ever shows in
 * the address bar and a reload of the page opens the board, not a sheet. */
function pageHistory(): BackHost {
  return {
    push: () => window.history.pushState({ kari: "layer" }, ""),
    back: () => window.history.back(),
    listen: (cb) => {
      window.addEventListener("popstate", cb);
      return () => window.removeEventListener("popstate", cb);
    },
  };
}

/** The one stack of this page. Null where there is no window, as in a test. */
export const backStack: BackStack | null = typeof window === "undefined" ? null : createBackStack(pageHistory());

/**
 * Close this layer on a back press while `active` is true.
 *
 * `close` is the same call the ✕ button makes, so a layer that asks before it
 * closes — a card sheet with an unsent prompt — asks here too. Such a layer
 * stays open and takes a new history entry, so the next press asks again.
 */
export function useBackClose(active: boolean, close: () => void) {
  const held = useRef(close);
  useEffect(() => {
    held.current = close;
  }, [close]);
  /** Counts the presses this layer refused. Each one asks for a new entry. */
  const [refused, setRefused] = useState(0);
  useEffect(() => {
    if (!active || !backStack) return;
    return backStack.push(() => {
      held.current();
      // The layer can refuse to close. React drops this update when the layer
      // did close, because the component is gone by then; when it stayed, the
      // update runs the effect again and pushes another entry.
      setRefused((n) => n + 1);
    });
  }, [active, refused]);
}
