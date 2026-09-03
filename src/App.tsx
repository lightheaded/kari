import { useCallback, useEffect, useMemo, useState } from "react";
import { api, onBoardChanged, onNotice } from "./api";
import type { Column, HubBoard, HubCard, Settings } from "./types";
import { Board, type Picked } from "./components/Board";
import { Drawer } from "./components/Drawer";
import { QuotaBar } from "./components/QuotaBar";
import { AddTaskModal, ColumnsModal, SettingsModal } from "./components/Modals";
import { ProposalPanel } from "./components/Proposals";
import { nodeDot } from "./util";

interface Toast {
  id: number;
  text: string;
  err?: boolean;
  card?: Picked | null;
}

const NODE_FILTER_KEY = "kari.nodeFilter";
/** Joins a node id and a project directory into one filter value. */
const PROJ_SEP = "\u0001";

function readNodeFilter(): string {
  try {
    return window.localStorage.getItem(NODE_FILTER_KEY) ?? "";
  } catch {
    return "";
  }
}

export default function App() {
  const [board, setBoard] = useState<HubBoard | null>(null);
  const [selected, setSelected] = useState<Picked | null>(null);
  const [query, setQuery] = useState("");
  const [project, setProject] = useState("");
  const [nodeFilter, setNodeFilter] = useState<string>(readNodeFilter);
  const [modal, setModal] = useState<"add" | "columns" | "settings" | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [planHidden, setPlanHidden] = useState<Set<string>>(() => new Set());

  const toast = useCallback((text: string, err = false, card: Picked | null = null) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t, { id, text, err, card }]);
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), err ? 7000 : card ? 8000 : 3500);
  }, []);

  const load = useCallback(async () => {
    try {
      const b = await api.board();
      setBoard(b);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
    api.settings().then(setSettings).catch(() => {});
    const un1 = onBoardChanged(load);
    const un2 = onNotice((n) => toast(`${n.title} — ${n.body}`, false, n.card_id ? { node: n.node_id, id: n.card_id } : null));
    const t = window.setInterval(load, 30000);
    return () => {
      un1();
      un2();
      window.clearInterval(t);
    };
  }, [load, toast]);

  useEffect(() => {
    try {
      window.localStorage.setItem(NODE_FILTER_KEY, nodeFilter);
    } catch {
      // a browser without storage keeps the filter for this run only
    }
  }, [nodeFilter]);

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

  const nodes = useMemo(() => board?.nodes ?? [], [board]);
  const manyNodes = nodes.length > 1;
  const nodeById = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);
  const localNodeId = nodes.find((n) => n.kind === "local")?.id ?? nodes[0]?.id ?? "local";
  /** A filter that names a node the board lost shows every node again. */
  const node = nodes.some((n) => n.id === nodeFilter) ? nodeFilter : "";

  const cwdOf = (c: HubCard) => c.card.project_cwd ?? c.session?.cwd ?? "";

  /** One entry per node and project directory. */
  const projects = useMemo(() => {
    const m = new Map<string, { node: string; cwd: string; name: string }>();
    for (const c of board?.cards ?? []) {
      const cwd = cwdOf(c);
      if (cwd && c.project_name) m.set(`${c.node_id}${PROJ_SEP}${cwd}`, { node: c.node_id, cwd, name: c.project_name });
    }
    return [...m.entries()].sort((a, b) => a[1].name.localeCompare(b[1].name));
  }, [board]);

  /** Project directories per node. The task dialog uses them until the node answers. */
  const projectsByNode = useMemo(() => {
    const out: Record<string, [string, string][]> = {};
    for (const [, p] of projects) (out[p.node] ??= []).push([p.cwd, p.name]);
    return out;
  }, [projects]);

  const filtered: HubCard[] = useMemo(() => {
    if (!board) return [];
    const q = query.trim().toLowerCase();
    return board.cards.filter((c) => {
      if (node && c.node_id !== node) return false;
      if (project && `${c.node_id}${PROJ_SEP}${cwdOf(c)}` !== project) return false;
      if (!q) return true;
      return (
        c.title.toLowerCase().includes(q) ||
        (c.project_name ?? "").toLowerCase().includes(q) ||
        (c.session?.last_prompt ?? "").toLowerCase().includes(q) ||
        (c.reason ?? "").toLowerCase().includes(q)
      );
    });
  }, [board, query, project, node]);

  const selectedCard = board?.cards.find((c) => selected && c.node_id === selected.node && c.card.id === selected.id) ?? null;
  const selectedOffline = selectedCard ? nodeById.get(selectedCard.node_id)?.online === false : false;
  const visibleColumns: Column[] = (board?.columns ?? []).filter((c) => !c.hidden).sort((a, b) => a.order - b.order);
  const needsMe = filtered.filter((c) => c.state === "needs_decision" || c.state === "needs_approval").length;
  const working = filtered.filter((c) => c.state === "working").length;
  const withQuota = (board?.quotas ?? []).filter((q) => q.quota);

  /** Show the plan again when the user asks for a new one on that node. */
  const unhidePlans = (nodeId: string) =>
    setPlanHidden((h) => new Set([...h].filter((k) => !k.startsWith(`${nodeId}:`))));

  return (
    <div className="app">
      <div className="topbar" data-tauri-drag-region>
        <div className="wordmark" data-tauri-drag-region>
          kari
        </div>
        <div className={`quotas ${withQuota.length > 1 ? "multi" : ""}`}>
          {withQuota.length === 0 ? (
            <QuotaBar quota={null} calibration={null} onHelp={() => setModal("settings")} />
          ) : (
            withQuota.map((q) => (
              <QuotaBar
                key={q.node_id}
                quota={q.quota}
                calibration={q.calibration}
                label={manyNodes ? q.node_name : undefined}
                onHelp={() => setModal("settings")}
                onFill={() => {
                  unhidePlans(q.node_id);
                  run(() => api.proposeNow(q.node_id), "Plan ready");
                }}
              />
            ))
          )}
        </div>
        <div className="spacer" data-tauri-drag-region />
        <span className={`pill ${board?.herdr_connected ? "on" : ""}`} title="herdr socket">
          <span className="dot" />
          herdr
        </span>
        <span className="pill" title="working / needs me">
          {working} working · {needsMe} need me
        </span>
        <button className="btn ghost" onClick={() => run(() => api.refresh(), "Refreshing")} title="Rescan now">
          ↻
        </button>
        <button className="btn ghost" onClick={() => setModal("columns")}>
          Columns
        </button>
        <button className="btn ghost" onClick={() => setModal("settings")}>
          Settings
        </button>
        <button className="btn primary" onClick={() => setModal("add")}>
          + Task
        </button>
      </div>

      <div className="filterbar">
        <input placeholder="Search title, prompt, project…" value={query} onChange={(e) => setQuery(e.target.value)} />
        <select value={project} onChange={(e) => setProject(e.target.value)}>
          <option value="">All projects</option>
          {projects.map(([key, p]) => (
            <option key={key} value={key}>
              {p.name}
              {manyNodes ? ` · ${nodeById.get(p.node)?.name ?? p.node}` : ""}
            </option>
          ))}
        </select>
        {manyNodes && (
          <div className="nodechips">
            <button className={`nodechip ${node === "" ? "sel" : ""}`} onClick={() => setNodeFilter("")}>
              All nodes
            </button>
            {nodes.map((n) => (
              <button
                key={n.id}
                className={`nodechip ${node === n.id ? "sel" : ""}`}
                title={n.error ?? (n.enabled ? (n.online ? "online" : "offline") : "disabled")}
                onClick={() => setNodeFilter(n.id)}
              >
                <span className={nodeDot(n)} />
                {n.name}
              </button>
            ))}
          </div>
        )}
        <span className="meta">
          {filtered.length} cards{board?.scanning ? " · scanning…" : ""}
          {board ? ` · updated ${new Date(board.generated_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}` : ""}
        </span>
        {error && <span className="meta" style={{ color: "var(--rust)" }}>{error}</span>}
      </div>

      {board ? (
        <Board
          columns={visibleColumns}
          cards={filtered}
          nodes={nodes}
          selected={selected}
          onSelect={(nodeId, id) => setSelected({ node: nodeId, id })}
          onMove={(nodeId, id, columnId) => run(() => api.moveCard(nodeId, id, columnId))}
          onJump={(nodeId, id) => run(() => api.jumpIn(nodeId, id), "Opened")}
        />
      ) : (
        <div className="empty">Loading the herd…</div>
      )}

      <div className="proposals">
        {(board?.proposals ?? [])
          .filter((p) => !planHidden.has(`${p.node_id}:${p.proposal.id}`))
          .map((p) => (
            <ProposalPanel
              key={`${p.node_id}:${p.proposal.id}`}
              proposal={p.proposal}
              nodeId={p.node_id}
              nodeName={manyNodes ? p.node_name : undefined}
              onClose={() => setPlanHidden((h) => new Set(h).add(`${p.node_id}:${p.proposal.id}`))}
              onAction={run}
              onSelectCard={(id) => setSelected({ node: p.node_id, id })}
            />
          ))}
      </div>

      {selectedCard && (
        <Drawer
          view={selectedCard}
          columns={board?.columns ?? []}
          settings={settings}
          showNode={manyNodes}
          offline={selectedOffline}
          onClose={() => setSelected(null)}
          onAction={run}
        />
      )}

      {modal === "add" && (
        <AddTaskModal
          nodes={nodes}
          defaultNode={node || localNodeId}
          projectsByNode={projectsByNode}
          onClose={() => setModal(null)}
          onSubmit={(nodeId, t) => run(() => api.addTask(nodeId, t), "Task added").then(() => setModal(null))}
        />
      )}
      {modal === "columns" && board && (
        <ColumnsModal
          columns={board.columns}
          onClose={() => setModal(null)}
          onSave={(cols) => run(() => api.setColumns(cols), "Columns saved").then(() => setModal(null))}
          onReset={() => run(() => api.resetColumns(), "Default columns restored").then(() => setModal(null))}
        />
      )}
      {modal === "settings" && settings && (
        <SettingsModal
          settings={settings}
          hooksInstalled={board?.hooks_installed ?? false}
          hooksPort={board?.hooks_port ?? settings.hooks_port}
          nodes={nodes}
          primary={board?.primary ?? true}
          onNodesChanged={load}
          onHooks={(install) => run(() => (install ? api.installHooks() : api.uninstallHooks()), install ? "Hooks installed" : "Hooks removed")}
          onClose={() => setModal(null)}
          onSave={(s) =>
            run(() => api.setSettings(s), "Settings saved").then(() => {
              setSettings(s);
              setModal(null);
            })
          }
          onStopAll={() => run(() => api.stopAll(), "Stopped kari jobs")}
        />
      )}

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
