import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, onBoardChanged, onConfirmQuit, onNotice } from "./api";
import { anyDirty } from "./dirty";
import type { BoardView, CardView, Column, Settings } from "./types";
import { Board } from "./components/Board";
import { Drawer } from "./components/Drawer";
import { QuotaBar } from "./components/QuotaBar";
import { AddTaskModal, ColumnsModal, SettingsModal } from "./components/Modals";
import { ProposalPanel } from "./components/Proposals";
import { noAutoCorrect } from "./util";

interface Toast {
  id: number;
  text: string;
  err?: boolean;
  cardId?: string | null;
  /** Milliseconds the toast stays. The timer pauses while the pointer is over it. */
  ttl: number;
}

/** One toast with its own clock, so a hover keeps it on screen for reading. */
function ToastItem({ t, onClose, onOpen }: { t: Toast; onClose: () => void; onOpen?: () => void }) {
  const left = useRef(t.ttl);
  const timer = useRef<number | null>(null);
  const startedAt = useRef(0);
  const start = useCallback(() => {
    startedAt.current = Date.now();
    timer.current = window.setTimeout(onClose, left.current);
  }, [onClose]);
  const pause = () => {
    if (timer.current == null) return;
    window.clearTimeout(timer.current);
    timer.current = null;
    left.current = Math.max(1500, left.current - (Date.now() - startedAt.current));
  };
  useEffect(() => {
    start();
    return () => {
      if (timer.current != null) window.clearTimeout(timer.current);
    };
  }, [start]);
  return (
    <div className={`toast ${t.err ? "err" : ""} ${onOpen ? "link" : ""}`} onMouseEnter={pause} onMouseLeave={start} onClick={onOpen} role="status">
      <span>{t.text}</span>
      <button
        className="x"
        aria-label="Dismiss"
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
      >
        ✕
      </button>
    </div>
  );
}

export default function App() {
  const [board, setBoard] = useState<BoardView | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [project, setProject] = useState("");
  const [modal, setModal] = useState<"add" | "columns" | "settings" | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [planHidden, setPlanHidden] = useState<string | null>(null);
  const [quitAsk, setQuitAsk] = useState(false);
  const [refreshingQuota, setRefreshingQuota] = useState(false);

  const toast = useCallback((text: string, err = false, cardId: string | null = null, ttl?: number) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t, { id, text, err, cardId, ttl: ttl ?? (err ? 9000 : cardId ? 10000 : 4000) }]);
  }, []);
  const dropToast = useCallback((id: number) => setToasts((all) => all.filter((x) => x.id !== id)), []);

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
    // Notices from the engine carry information the user did not ask for. They stay long enough to read.
    const un2 = onNotice((n) => toast(`${n.title} — ${n.body}`, false, n.card_id, 15000));
    const un3 = onConfirmQuit(() => setQuitAsk(true));
    const t = window.setInterval(load, 30000);
    // A reload from the dev server or a navigation: warn while a form holds input.
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

  /** Run an action, toast the result, reload the board. Resolves true on success. */
  const run = useCallback(
    async (fn: () => Promise<unknown>, ok?: string): Promise<boolean> => {
      try {
        const r = await fn();
        if (ok) toast(typeof r === "string" && r ? r : ok);
        await load();
        return true;
      } catch (e) {
        toast(String(e), true);
        return false;
      }
    },
    [load, toast],
  );
  const runVoid = useCallback((fn: () => Promise<unknown>, ok?: string) => run(fn, ok).then(() => {}), [run]);

  const projects = useMemo(() => {
    const m = new Map<string, string>();
    for (const c of board?.cards ?? []) {
      const cwd = c.card.project_cwd ?? c.session?.cwd ?? null;
      if (cwd && c.project_name) m.set(cwd, c.project_name);
    }
    return [...m.entries()].sort((a, b) => a[1].localeCompare(b[1]));
  }, [board]);

  const filtered: CardView[] = useMemo(() => {
    if (!board) return [];
    const q = query.trim().toLowerCase();
    return board.cards.filter((c) => {
      if (project) {
        const cwd = c.card.project_cwd ?? c.session?.cwd ?? "";
        if (cwd !== project) return false;
      }
      if (!q) return true;
      return (
        c.title.toLowerCase().includes(q) ||
        (c.project_name ?? "").toLowerCase().includes(q) ||
        (c.session?.last_prompt ?? "").toLowerCase().includes(q) ||
        (c.reason ?? "").toLowerCase().includes(q)
      );
    });
  }, [board, query, project]);

  const selectedCard = board?.cards.find((c) => c.card.id === selected) ?? null;
  const visibleColumns: Column[] = (board?.columns ?? []).filter((c) => !c.hidden).sort((a, b) => a.order - b.order);
  const needsMe = board?.cards.filter((c) => c.state === "needs_decision" || c.state === "needs_approval").length ?? 0;
  const working = board?.cards.filter((c) => c.state === "working").length ?? 0;
  const proposal = board?.proposal ?? null;
  const planOpen = !!proposal && proposal.id !== planHidden;

  return (
    <div className="app">
      <div className="topbar" data-tauri-drag-region>
        <div className="wordmark" data-tauri-drag-region>
          kari
        </div>
        <QuotaBar
          quota={board?.quota ?? null}
          calibration={board?.calibration ?? null}
          onHelp={() => setModal("settings")}
          refreshing={refreshingQuota}
          onRefresh={async () => {
            setRefreshingQuota(true);
            await run(() => api.fetchUsageNow(), "Quota refreshed from the usage endpoint");
            setRefreshingQuota(false);
          }}
          onFill={() => {
            setPlanHidden(null);
            run(() => api.proposeNow(), "Plan ready");
          }}
        />
        <div className="spacer" data-tauri-drag-region />
        {proposal && !planOpen && (
          <button className="pill plan" onClick={() => setPlanHidden(null)} title="Show the plan panel again">
            <span className="dot" />
            plan · {proposal.items.length}
          </button>
        )}
        <span className={`pill ${board?.herdr_connected ? "on" : ""}`} title="herdr socket">
          <span className="dot" />
          herdr
        </span>
        <span className="pill" title="Sessions where Claude works now, and sessions that wait for your decision or approval">
          {working} working · {needsMe} need me
        </span>
        <button className="btn ghost" onClick={() => runVoid(() => api.refresh(), "Refreshing")} title="Rescan now">
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
        <input {...noAutoCorrect} placeholder="Search title, prompt, project…" value={query} onChange={(e) => setQuery(e.target.value)} />
        <select value={project} onChange={(e) => setProject(e.target.value)}>
          <option value="">All projects</option>
          {projects.map(([cwd, name]) => (
            <option key={cwd} value={cwd}>
              {name}
            </option>
          ))}
        </select>
        <span className="meta">
          {filtered.length} cards{board?.scanning ? " · scanning…" : ""}
          {board ? ` · updated ${new Date(board.generated_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}` : ""}
        </span>
        {error && <span className="meta" style={{ color: "var(--rust)" }}>{error}</span>}
      </div>

      <div className="main">
        {board ? (
          <Board
            columns={visibleColumns}
            cards={filtered}
            selected={selected}
            onSelect={setSelected}
            onMove={(cardId, columnId) => runVoid(() => api.moveCard(cardId, columnId))}
            onReorder={(ids) => runVoid(() => api.reorderCards(ids))}
            onJump={(cardId) => runVoid(() => api.jumpIn(cardId), "Opened")}
          />
        ) : (
          <div className="empty">Loading the herd…</div>
        )}

        {proposal && planOpen && (
          <ProposalPanel key={proposal.id} proposal={proposal} onClose={() => setPlanHidden(proposal.id)} onAction={runVoid} onSelectCard={(id) => setSelected(id)} />
        )}
      </div>

      {selectedCard && (
        <Drawer
          view={selectedCard}
          columns={board?.columns ?? []}
          settings={settings}
          onClose={() => setSelected(null)}
          onAction={runVoid}
        />
      )}

      {modal === "add" && (
        <AddTaskModal
          projects={projects}
          onClose={() => setModal(null)}
          onSubmit={(t) =>
            run(() => api.addTask(t), "Task added").then((ok) => {
              if (ok) setModal(null);
              return ok;
            })
          }
        />
      )}
      {modal === "columns" && board && (
        <ColumnsModal
          columns={board.columns}
          onClose={() => setModal(null)}
          onSave={(cols) => run(() => api.setColumns(cols), "Columns saved").then((ok) => ok && setModal(null))}
          onReset={() => run(() => api.resetColumns(), "Default columns restored").then((ok) => ok && setModal(null))}
        />
      )}
      {modal === "settings" && settings && (
        <SettingsModal
          settings={settings}
          hooksInstalled={board?.hooks_installed ?? false}
          hooksPort={board?.hooks_port ?? settings.hooks_port}
          onHooks={(install) => runVoid(() => (install ? api.installHooks() : api.uninstallHooks()), install ? "Hooks installed" : "Hooks removed")}
          onClose={() => setModal(null)}
          onSave={(s) =>
            run(() => api.setSettings(s), "Settings saved").then((ok) => {
              if (!ok) return;
              setSettings(s);
              setModal(null);
            })
          }
          onStopAll={() => runVoid(() => api.stopAll(), "Stopped kari jobs")}
        />
      )}

      {quitAsk && (
        <div className="backdrop" onMouseDown={(e) => e.target === e.currentTarget && setQuitAsk(false)}>
          <div className="modal narrow" role="alertdialog" aria-label="Quit kari?">
            <header>
              <h3>Quit kari?</h3>
            </header>
            <div className="body">
              <p>A form holds unsaved input. A new task draft is kept and comes back when you open the form again. Other edits are lost.</p>
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

      <div className="toasts">
        {toasts.map((t) => (
          <ToastItem
            key={t.id}
            t={t}
            onClose={() => dropToast(t.id)}
            onOpen={
              t.cardId
                ? () => {
                    setSelected(t.cardId!);
                    dropToast(t.id);
                  }
                : undefined
            }
          />
        ))}
      </div>
    </div>
  );
}
