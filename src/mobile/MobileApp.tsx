import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, onBoardChanged, onNotice } from "../api";
import type { HubBoard, HubCard, Project, Settings } from "../types";
import type { Picked } from "../components/Board";
import { Drawer } from "../components/Drawer";
import { AddTaskModal } from "../components/Modals";
import { Inbox } from "./Inbox";
import { BoardTab } from "./BoardTab";
import { NodesTab } from "./NodesTab";
import "./mobile.css";

type Tab = "inbox" | "board" | "add" | "nodes";

/** A call that never answers must not hold the first screen forever.
 *
 * The web view of the app can send its first request before the Rust side
 * finished opening its store, and such a request is answered by nobody: the
 * promise stays pending, so no error ever arrives. A deadline turns that into
 * a failure the retry below can act on. */
function within<T>(p: Promise<T>, secs: number, what: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = window.setTimeout(() => reject(new Error(`${what} did not answer in ${secs}s`)), secs * 1000);
    p.then(
      (v) => {
        window.clearTimeout(timer);
        resolve(v);
      },
      (e) => {
        window.clearTimeout(timer);
        reject(e);
      },
    );
  });
}

/** Stands in until the first board arrives, so pairing works at once. */
const EMPTY_BOARD: HubBoard = {
  columns: [],
  hub_id: "",
  hub_name: "",
  primary: false,
  nodes: [],
  cards: [],
  quotas: [],
  queues: [],
  proposals: [],
  generated_at: "",
  scanning: false,
  herdr_connected: false,
  hooks_installed: false,
  hooks_port: 47311,
};

interface Toast {
  id: number;
  text: string;
  err?: boolean;
  card?: Picked | null;
}

/** The phone: four tabs, one card sheet, the same commands as the desktop. */
export default function MobileApp() {
  const [board, setBoard] = useState<HubBoard | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [tab, setTab] = useState<Tab>("inbox");
  const [selected, setSelected] = useState<Picked | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [waited, setWaited] = useState(0);

  const toast = useCallback((text: string, err = false, card: Picked | null = null) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t, { id, text, err, card }]);
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), err ? 7000 : card ? 8000 : 3500);
  }, []);

  // The hub may still be opening its store when the first call goes out. A
  // failed call comes back in a moment, not at the next poll thirty seconds
  // later, which is what made the first screen look stuck.
  const retry = useRef<number | null>(null);
  const load = useCallback(
    async function run(attempt = 0): Promise<void> {
      try {
        const b = await within(api.board(), 6, "the board");
        setBoard(b);
        setError(null);
      } catch (e) {
        setError(String(e));
        if (retry.current !== null) window.clearTimeout(retry.current);
        retry.current = window.setTimeout(() => void run(attempt + 1), Math.min(8000, 400 * 2 ** attempt));
      }
    },
    [],
  );

  const loadSettings = useCallback(
    () =>
      within(api.settings(), 6, "the settings")
        .then(setSettings)
        .catch(() => {}),
    [],
  );

  useEffect(() => {
    load();
    loadSettings();
    const un1 = onBoardChanged(load);
    const un2 = onNotice((n) => toast(`${n.title} — ${n.body}`, false, n.card_id ? { node: n.node_id, id: n.card_id } : null));
    const t = window.setInterval(load, 30000);
    return () => {
      un1();
      un2();
      window.clearInterval(t);
      if (retry.current !== null) window.clearTimeout(retry.current);
    };
  }, [load, loadSettings, toast]);

  const run = useCallback(
    async (fn: () => Promise<unknown>, ok?: string) => {
      try {
        const r = await fn();
        if (ok) toast(typeof r === "string" && r ? r : ok);
        await load();
      } catch (e) {
        toast(String(e), true);
      }
    },
    [load, toast],
  );

  useEffect(() => {
    if (board) return;
    const t = window.setInterval(() => setWaited((w) => w + 1), 1000);
    return () => window.clearInterval(t);
  }, [board]);

  const nodes = useMemo(() => board?.nodes ?? [], [board]);
  const nodeById = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);
  const cards: HubCard[] = useMemo(() => board?.cards ?? [], [board]);
  const selectedCard = cards.find((c) => selected && c.node_id === selected.node && c.card.id === selected.id) ?? null;
  const selectedOffline = selectedCard ? nodeById.get(selectedCard.node_id)?.online === false : false;
  const needsMe = cards.filter((c) => c.state === "needs_decision" || c.state === "needs_approval").length;

  /** Project directories per node, from the cards. The task form uses them until the node answers. */
  const projectsByNode = useMemo(() => {
    const out: Record<string, Project[]> = {};
    const seen = new Set<string>();
    for (const c of cards) {
      const cwd = c.card.project_cwd ?? c.session?.cwd;
      if (!cwd || !c.project_name || seen.has(`${c.node_id}|${cwd}`)) continue;
      seen.add(`${c.node_id}|${cwd}`);
      (out[c.node_id] ??= []).push({ cwd, name: c.project_name });
    }
    return out;
  }, [cards]);

  const open = (node: string, id: string) => setSelected({ node, id });

  return (
    <div className="mapp">
      <main className="mmain">
        {error && <div className="merr">{error}</div>}
        {!board && !error && tab !== "nodes" && <div className="empty">Loading the herd… {waited > 2 ? `${waited}s` : ""}</div>}
        {board && tab === "inbox" && <Inbox board={board} onOpen={open} onAction={run} />}
        {board && tab === "board" && <BoardTab board={board} onOpen={open} onAction={run} />}
        {tab === "nodes" && <NodesTab board={board ?? EMPTY_BOARD} settings={settings} onChanged={load} onSettingsChanged={loadSettings} onAction={run} />}
        {tab === "add" && board && (
          <AddTaskModal
            nodes={nodes}
            defaultNode={nodes.find((n) => n.online)?.id ?? nodes[0]?.id ?? ""}
            defaultProject={null}
            columnId={null}
            columns={board.columns}
            projectsByNode={projectsByNode}
            onClose={() => setTab("inbox")}
            onSubmit={(nodeId, t) =>
              run(() => api.addTask(nodeId, t), "Task added").then(() => {
                setTab("board");
              })
            }
          />
        )}
      </main>

      {selectedCard && (
        <Drawer
          view={selectedCard}
          columns={board?.columns ?? []}
          settings={settings}
          showNode={nodes.length > 1}
          offline={selectedOffline}
          mobile
          onClose={() => setSelected(null)}
          onAction={run}
        />
      )}

      <nav className="mtabs" aria-label="Sections">
        {(
          [
            ["inbox", "Needs you", needsMe],
            ["board", "Board", 0],
            ["add", "Add", 0],
            ["nodes", "Nodes", nodes.filter((n) => n.enabled && !n.online).length],
          ] as [Tab, string, number][]
        ).map(([id, label, badge]) => (
          <button key={id} className={`mtab ${tab === id ? "on" : ""}`} onClick={() => setTab(id)}>
            {label}
            {badge > 0 && <span className={`mbadge ${id === "nodes" ? "off" : ""}`}>{badge}</span>}
          </button>
        ))}
      </nav>

      <div className="toasts">
        {toasts.map((t) => (
          <div
            key={t.id}
            className={`toast ${t.err ? "err" : ""} ${t.card ? "link" : ""}`}
            onClick={() => {
              if (t.card) {
                setSelected(t.card);
                setToasts((all) => all.filter((x) => x.id !== t.id));
              }
            }}
          >
            {t.text}
          </div>
        ))}
      </div>
    </div>
  );
}
