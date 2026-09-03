import { useCallback, useEffect, useMemo, useState } from "react";
import { api, onBoardChanged, onNotice } from "./api";
import type { BoardView, CardView, Column, Settings } from "./types";
import { Board } from "./components/Board";
import { Drawer } from "./components/Drawer";
import { QuotaBar } from "./components/QuotaBar";
import { AddTaskModal, ColumnsModal, SettingsModal } from "./components/Modals";
import { ProposalPanel } from "./components/Proposals";

interface Toast {
  id: number;
  text: string;
  err?: boolean;
  cardId?: string | null;
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

  const toast = useCallback((text: string, err = false, cardId: string | null = null) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t, { id, text, err, cardId }]);
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), err ? 7000 : cardId ? 8000 : 3500);
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
    const un2 = onNotice((n) => toast(`${n.title} — ${n.body}`, false, n.card_id));
    const t = window.setInterval(load, 30000);
    return () => {
      un1();
      un2();
      window.clearInterval(t);
    };
  }, [load, toast]);

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
          onFill={() => {
            setPlanHidden(null);
            run(() => api.proposeNow(), "Plan ready");
          }}
        />
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

      {board ? (
        <Board
          columns={visibleColumns}
          cards={filtered}
          selected={selected}
          onSelect={setSelected}
          onMove={(cardId, columnId) => run(() => api.moveCard(cardId, columnId))}
          onJump={(cardId) => run(() => api.jumpIn(cardId), "Opened")}
        />
      ) : (
        <div className="empty">Loading the herd…</div>
      )}

      {board?.proposal && board.proposal.id !== planHidden && (
        <ProposalPanel
          proposal={board.proposal}
          onClose={() => setPlanHidden(board.proposal!.id)}
          onAction={run}
          onSelectCard={(id) => setSelected(id)}
        />
      )}

      {selectedCard && (
        <Drawer
          view={selectedCard}
          columns={board?.columns ?? []}
          settings={settings}
          onClose={() => setSelected(null)}
          onAction={run}
        />
      )}

      {modal === "add" && (
        <AddTaskModal
          projects={projects}
          onClose={() => setModal(null)}
          onSubmit={(t) => run(() => api.addTask(t), "Task added").then(() => setModal(null))}
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
            className={`toast ${t.err ? "err" : ""} ${t.cardId ? "link" : ""}`}
            onClick={() => {
              if (t.cardId) {
                setSelected(t.cardId);
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
