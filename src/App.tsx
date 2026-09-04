import { useCallback, useEffect, useMemo, useState } from "react";
import { api, onBoardChanged, onConfirmQuit, onNotice } from "./api";
import type { AutomationMode, Column, HubBoard, HubCard, Project, Settings } from "./types";
import { AUTOMATION_MODES } from "./types";
import { Board, type Picked, type Reorder } from "./components/Board";
import { Drawer } from "./components/Drawer";
import { StatsStrip } from "./components/StatsStrip";
import { AutomationSwitch } from "./components/AutomationSwitch";
import { QueueStrip } from "./components/QueueStrip";
import { ProjectPicker, type PickerItem } from "./components/ProjectPicker";
import { AddTaskModal, ColumnsModal, SettingsModal } from "./components/Modals";
import { ProposalPanel } from "./components/Proposals";
import { Toasts } from "./components/Toasts";
import { useToasts, type Undo } from "./toasts";
import { useSticky } from "./hooks";
import { anyDirty } from "./dirty";
import { noAutoFill } from "./util";

/** Joins a node id and a project directory into one filter value. */
const PROJ_SEP = "\u0001";

export default function App() {
  const [board, setBoard] = useState<HubBoard | null>(null);
  const [selected, setSelected] = useState<Picked | null>(null);
  const [query, setQuery] = useState("");
  const [project, setProject] = useState("");
  const [nodeFilter, setNodeFilter] = useSticky<string>("kari.nodeFilter", "");
  /** The project the last task went to. Used when no filter names one. */
  const [lastProject, setLastProject] = useSticky<string>("kari.lastProject", "");
  const [queueOpen, setQueueOpen] = useSticky<boolean>("kari.queueOpen", false);
  const [modal, setModal] = useState<"add" | "columns" | "settings" | null>(null);
  /** Column a new task must land in, when the dialog came from a column foot. */
  const [addColumn, setAddColumn] = useState<string | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [planHidden, setPlanHidden] = useState<Set<string>>(() => new Set());
  /** The tray or Cmd+Q asked to quit while a form holds unsaved input. */
  const [quitAsk, setQuitAsk] = useState(false);
  const [refreshingQuota, setRefreshingQuota] = useState(false);

  const { toasts, toast, drop: dropToast, clear: clearToasts } = useToasts();

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
    const un2 = onNotice((n) =>
      toast(`${n.title} — ${n.body}`, { card: n.card_id ? { node: n.node_id, id: n.card_id } : null, ttl: 20000 }),
    );
    const un3 = onConfirmQuit(() => setQuitAsk(true));
    const t = window.setInterval(load, 30000);
    // A reload from the dev server, or a navigation: warn while a form holds input.
    const onUnload = (e: BeforeUnloadEvent) => {
      if (anyDirty()) {
        e.preventDefault();
        e.returnValue = "";
      }
    };
    window.addEventListener("beforeunload", onUnload);
    return () => {
      un1();
      un2();
      un3();
      window.clearInterval(t);
      window.removeEventListener("beforeunload", onUnload);
    };
  }, [load, toast]);

  /** Run one action, report it, and offer its undo when it has one. `undo`
   *  can read the result, for example the card a new task became. */
  const run = useCallback(
    async <T,>(fn: () => Promise<T>, ok?: string, undo?: Undo | ((r: T) => Undo | null)) => {
      try {
        const r = await fn();
        if (ok) {
          const u = typeof undo === "function" ? undo(r) : undo;
          toast(typeof r === "string" && r ? r : ok, { undo: u ?? undefined });
        }
        await load();
      } catch (e) {
        toast(String(e), { err: true });
      }
    },
    [load, toast],
  );

  /** The user pressed Undo. The reversal is an action like any other. */
  const undo = useCallback((u: Undo) => void run(u.run, u.done), [run]);

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

  const projectItems: PickerItem[] = useMemo(
    () =>
      projects.map(([key, p]) => ({
        value: key,
        label: p.name,
        hint: manyNodes ? `${nodeById.get(p.node)?.name ?? p.node} · ${p.cwd}` : p.cwd,
      })),
    [projects, manyNodes, nodeById],
  );

  /** Project directories per node. The task dialog uses them until the node answers. */
  const projectsByNode = useMemo(() => {
    const out: Record<string, Project[]> = {};
    for (const [, p] of projects) (out[p.node] ??= []).push({ cwd: p.cwd, name: p.name });
    return out;
  }, [projects]);

  /** The project a new task starts in: the filter first, then the last one used. */
  const addNode = node || localNodeId;
  const addProject = useMemo(() => {
    const pick = (key: string) => {
      const p = projects.find(([k]) => k === key);
      return p && (!node || p[1].node === node) ? p[1] : null;
    };
    return (pick(project) ?? pick(lastProject))?.cwd ?? null;
  }, [project, lastProject, projects, node]);

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
  const queues = (board?.queues ?? []).filter((q) => !node || q.node_id === node);
  const plans = (board?.proposals ?? []).filter((p) => !node || p.node_id === node);
  const openPlans = plans.filter((p) => !planHidden.has(`${p.node_id}:${p.proposal.id}`));
  const hiddenPlans = plans.length - openPlans.length;

  /** Show the plan again when the user asks for a new one on that node. */
  const unhidePlans = (nodeId: string) =>
    setPlanHidden((h) => new Set([...h].filter((k) => !k.startsWith(`${nodeId}:`))));

  /** Remember the project of a task the user just added. */
  const rememberProject = (nodeId: string, cwd: string | null) => {
    if (cwd) setLastProject(`${nodeId}${PROJ_SEP}${cwd}`);
  };

  /** The undo of a column save: put the columns that were there back. */
  const undoColumns = (was: Column[]): Undo | undefined =>
    was.length ? { done: "Columns put back", run: () => api.setColumns(was) } : undefined;

  /** The undo of a settings save. The screen holds the old settings too. */
  const undoSettings = (was: Settings | null): Undo | undefined =>
    was
      ? {
          done: "Settings put back",
          run: () => api.setSettings(was).then(() => setSettings(was)),
        }
      : undefined;

  /** A card dropped in another column. Say where it went, and offer the way back. */
  const moveCard = (nodeId: string, id: string, columnId: string) => {
    const was = board?.cards.find((c) => c.node_id === nodeId && c.card.id === id)?.column_id;
    const name = (board?.columns ?? []).find((k) => k.id === columnId)?.name ?? "another column";
    run(() => api.moveCard(nodeId, id, columnId), `Moved to ${name}`, () =>
      was && was !== columnId ? { done: "Card moved back", run: () => api.moveCard(nodeId, id, was) } : null,
    );
  };

  /** The automation switch. An empty node id means every node that answers, so
   *  the undo holds only when the nodes agreed on one mode before. */
  const setMode = (nodeId: string, mode: AutomationMode) => {
    const scope = nodeId ? nodes.filter((n) => n.id === nodeId) : nodes.filter((n) => n.enabled && n.online);
    const modes = new Set(scope.map((n) => n.automation_mode || "ask"));
    const was = modes.size === 1 ? ([...modes][0] as AutomationMode) : null;
    const label = (m: AutomationMode) => AUTOMATION_MODES.find((x) => x.value === m)?.label ?? m;
    run(() => api.setAutomationMode(nodeId, mode), `Automation: ${label(mode)}`, () =>
      was && was !== mode ? { done: `Automation back to ${label(was)}`, run: () => api.setAutomationMode(nodeId, was) } : null,
    );
  };

  /** A one-line task from the foot of a column. */
  const addInline = async (columnId: string, title: string) => {
    await run(
      () =>
        api.addTask(addNode, {
          title,
          project_cwd: addProject,
          run_prompt: null,
          auto_run: false,
          priority: 0,
          notes: null,
          model: null,
          column_id: columnId,
        }),
      "Task added",
    );
    rememberProject(addNode, addProject);
  };

  return (
    <div className="app">
      <div className="topbar" data-tauri-drag-region>
        <div className="wordmark" data-tauri-drag-region>
          kari
        </div>
        <div className="spacer" data-tauri-drag-region />
        <AutomationSwitch
          nodes={nodes}
          filter={node}
          onChange={(nodeId, mode: AutomationMode) => setMode(nodeId, mode)}
        />
        {hiddenPlans > 0 && (
          <button className="pill plan" onClick={() => setPlanHidden(new Set())} title="Show the plan panel again">
            <span className="dot" />
            plan · {hiddenPlans}
          </button>
        )}
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
        <button
          className="btn primary"
          onClick={() => {
            setAddColumn(null);
            setModal("add");
          }}
        >
          + Task
        </button>
      </div>

      <StatsStrip
        quotas={board?.quotas ?? []}
        nodes={nodes}
        filter={node}
        onFilter={setNodeFilter}
        onFill={(nodeId) => {
          unhidePlans(nodeId);
          run(() => api.proposeNow(nodeId), "Plan ready");
        }}
        onHelp={() => setModal("settings")}
        refreshing={refreshingQuota}
        onRefresh={async () => {
          setRefreshingQuota(true);
          await run(() => api.fetchUsageNow(), "Quota read from the usage endpoint");
          setRefreshingQuota(false);
        }}
      />

      <div className="filterbar">
        <input {...noAutoFill} placeholder="Search title, prompt, project…" value={query} onChange={(e) => setQuery(e.target.value)} />
        <ProjectPicker
          items={projectItems}
          value={project}
          allLabel="All projects"
          ariaLabel="Filter by project"
          onChange={setProject}
        />
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
                onClick={() => setNodeFilter(node === n.id ? "" : n.id)}
              >
                <span className={n.enabled ? (n.online ? "dot online" : "dot offline") : "dot disabled"} />
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

      <QueueStrip
        queues={queues}
        showNode={manyNodes}
        open={queueOpen}
        onToggle={() => setQueueOpen(!queueOpen)}
        onSelectCard={(n, id) => setSelected({ node: n, id })}
      />

      <div className="main">
        {board ? (
          <Board
            columns={visibleColumns}
            cards={filtered}
            nodes={nodes}
            selected={selected}
            onSelect={(nodeId, id) => setSelected({ node: nodeId, id })}
            onMove={(nodeId, id, columnId) => moveCard(nodeId, id, columnId)}
            onReorder={(r: Reorder) => run(() => api.reorderCards(r.node, r.ranked, r.unranked))}
            onJump={(nodeId, id) => run(() => api.jumpIn(nodeId, id), "Opened")}
            onFilterNode={(nodeId) => setNodeFilter(node === nodeId ? "" : nodeId)}
            onAdd={addInline}
            onAddFull={(columnId) => {
              setAddColumn(columnId);
              setModal("add");
            }}
          />
        ) : (
          <div className="empty">Loading the herd…</div>
        )}

        {openPlans.length > 0 && (
          <div className="planrail">
            {openPlans.map((p) => (
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
        )}
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
          defaultNode={addNode}
          defaultProject={addProject}
          columnId={addColumn}
          columns={board?.columns ?? []}
          projectsByNode={projectsByNode}
          onClose={() => setModal(null)}
          onSubmit={(nodeId, t) =>
            run(() => api.addTask(nodeId, t), "Task added", (c) => ({
              done: "Task taken off the board",
              run: () => api.deleteCard(nodeId, c.id),
            })).then(() => {
              rememberProject(nodeId, t.project_cwd);
              setModal(null);
            })
          }
        />
      )}
      {modal === "columns" && board && (
        <ColumnsModal
          columns={board.columns}
          onClose={() => setModal(null)}
          onSave={(cols) => run(() => api.setColumns(cols), "Columns saved", undoColumns(board.columns)).then(() => setModal(null))}
          onReset={() =>
            run(() => api.resetColumns(), "Default columns restored", undoColumns(board.columns)).then(() => setModal(null))
          }
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
          onHooks={(install) =>
            run<string | void>(() => (install ? api.installHooks() : api.uninstallHooks()), install ? "Hooks installed" : "Hooks removed", {
              done: install ? "Hooks removed again" : "Hooks installed again",
              run: () => (install ? api.uninstallHooks() : api.installHooks()),
            })
          }
          onClose={() => setModal(null)}
          onSave={(s) =>
            run(() => api.setSettings(s), "Settings saved", undoSettings(settings)).then(() => {
              setSettings(s);
              setModal(null);
            })
          }
          onSaveNow={(s) => run(() => api.setSettings(s), "Settings saved", undoSettings(settings)).then(() => setSettings(s))}
          onStopAll={() => run(() => api.stopAll(), "Stopped kari jobs")}
        />
      )}

      {quitAsk && (
        <div className="backdrop" onMouseDown={(e) => e.target === e.currentTarget && setQuitAsk(false)}>
          <div className="modal narrow" role="alertdialog" aria-label="Quit kari?">
            <header>
              <h3>Quit kari?</h3>
            </header>
            <div className="body">
              <p>A form holds unsaved input. Quitting now throws those edits away.</p>
            </div>
            <footer>
              <button className="btn" onClick={() => setQuitAsk(false)} autoFocus>
                Keep working
              </button>
              <button className="btn danger" onClick={() => api.quitNow()}>
                Quit anyway
              </button>
            </footer>
          </div>
        </div>
      )}

      <Toasts toasts={toasts} onDrop={dropToast} onClear={clearToasts} onOpen={setSelected} onUndo={undo} />
    </div>
  );
}
