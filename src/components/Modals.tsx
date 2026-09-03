import { useEffect, useState } from "react";
import { api } from "../api";
import type { Column, DerivedState, NewTask, Settings } from "../types";
import { ALL_STATES, RUN_MODELS, STATE_LABEL } from "../types";

function Modal({ title, children, footer, onClose }: { title: string; children: React.ReactNode; footer?: React.ReactNode; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-label={title}>
        <header>
          <h3>{title}</h3>
          <div className="spacer" />
          <button className="btn ghost sm" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </header>
        <div className="body">{children}</div>
        {footer && <footer>{footer}</footer>}
      </div>
    </div>
  );
}

export function AddTaskModal({ projects, onClose, onSubmit }: { projects: [string, string][]; onClose: () => void; onSubmit: (t: NewTask) => void }) {
  const [title, setTitle] = useState("");
  const [cwd, setCwd] = useState(projects[0]?.[0] ?? "");
  const [custom, setCustom] = useState("");
  const [prompt, setPrompt] = useState("");
  const [autoRun, setAutoRun] = useState(false);
  const [priority, setPriority] = useState(0);
  const [notes, setNotes] = useState("");
  const [model, setModel] = useState("");
  const dir = cwd === "__custom" ? custom : cwd;
  return (
    <Modal
      title="New task"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn primary"
            disabled={!title.trim()}
            onClick={() =>
              onSubmit({
                title: title.trim(),
                project_cwd: dir || null,
                run_prompt: prompt.trim() || null,
                auto_run: autoRun,
                priority,
                notes: notes.trim() || null,
                model: model || null,
              })
            }
          >
            Add
          </button>
        </>
      }
    >
      <div className="field">
        <label>Title</label>
        <input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} placeholder="What needs to happen" />
      </div>
      <div className="field">
        <label>Project directory</label>
        <select value={cwd} onChange={(e) => setCwd(e.target.value)}>
          {projects.map(([c, n]) => (
            <option key={c} value={c}>
              {n} — {c}
            </option>
          ))}
          <option value="__custom">Other path…</option>
        </select>
        {cwd === "__custom" && <input value={custom} onChange={(e) => setCustom(e.target.value)} placeholder="/absolute/path" />}
      </div>
      <div className="field">
        <label>Run prompt (what Claude gets when the task starts)</label>
        <textarea value={prompt} onChange={(e) => setPrompt(e.target.value)} placeholder="Leave empty to use the title" />
      </div>
      <div className="grid2">
        <div className="field">
          <label>Priority</label>
          <input type="number" value={priority} onChange={(e) => setPriority(Number(e.target.value))} />
        </div>
        <div className="field">
          <label>Model (optional)</label>
          <select value={model} onChange={(e) => setModel(e.target.value)}>
            {RUN_MODELS.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </div>
      </div>
      <label className="field inline">
        <input type="checkbox" checked={autoRun} onChange={(e) => setAutoRun(e.target.checked)} />
        <span>May run unattended</span>
      </label>
      <div className="field">
        <label>Notes</label>
        <textarea value={notes} onChange={(e) => setNotes(e.target.value)} />
      </div>
    </Modal>
  );
}

const COLORS = ["neutral", "green", "amber", "rust", "slate"];

export function ColumnsModal({ columns, onClose, onSave, onReset }: { columns: Column[]; onClose: () => void; onSave: (c: Column[]) => void; onReset: () => void }) {
  const [cols, setCols] = useState<Column[]>(() => [...columns].sort((a, b) => a.order - b.order).map((c) => ({ ...c })));
  const update = (i: number, patch: Partial<Column>) => setCols((cs) => cs.map((c, j) => (j === i ? { ...c, ...patch } : c)));
  const move = (i: number, d: number) =>
    setCols((cs) => {
      const n = [...cs];
      const j = i + d;
      if (j < 0 || j >= n.length) return cs;
      [n[i], n[j]] = [n[j], n[i]];
      return n.map((c, k) => ({ ...c, order: k }));
    });
  const toggleState = (i: number, s: DerivedState) =>
    update(i, { accepts: cols[i].accepts.includes(s) ? cols[i].accepts.filter((x) => x !== s) : [...cols[i].accepts, s] });
  const unassigned = ALL_STATES.filter((s) => s !== "stale" && !cols.some((c) => !c.hidden && c.accepts.includes(s)));
  return (
    <Modal
      title="Columns"
      onClose={onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onReset}>
            Reset to defaults
          </button>
          <div className="spacer" />
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" onClick={() => onSave(cols.map((c, k) => ({ ...c, order: k })))}>
            Save
          </button>
        </>
      }
    >
      <div className="hint">
        Each column accepts a set of derived states. A state that no visible column accepts falls back to the column that accepts Unknown. Stale cards are hidden unless a column
        accepts Stale.
      </div>
      {unassigned.length > 0 && <div className="hint" style={{ color: "var(--amber)" }}>Not shown anywhere: {unassigned.map((s) => STATE_LABEL[s]).join(", ")}</div>}
      {cols.map((c, i) => (
        <div key={c.id} className="colrow">
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <button className="btn ghost sm" onClick={() => move(i, -1)} aria-label="Up">
              ↑
            </button>
            <button className="btn ghost sm" onClick={() => move(i, 1)} aria-label="Down">
              ↓
            </button>
          </div>
          <input type="text" value={c.name} onChange={(e) => update(i, { name: e.target.value })} />
          <input type="number" placeholder="WIP" value={c.wip_limit ?? ""} onChange={(e) => update(i, { wip_limit: e.target.value === "" ? null : Number(e.target.value) })} title="WIP limit" />
          <select value={c.color ?? "neutral"} onChange={(e) => update(i, { color: e.target.value })}>
            {COLORS.map((k) => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </select>
          <div style={{ display: "flex", gap: 4 }}>
            <button className={`toggle ${c.hidden ? "" : "on"}`} onClick={() => update(i, { hidden: !c.hidden })}>
              {c.hidden ? "hidden" : "shown"}
            </button>
            <button className="btn ghost sm" onClick={() => setCols((cs) => cs.filter((_, j) => j !== i))} aria-label="Delete column">
              ✕
            </button>
          </div>
          <div className="accepts">
            {ALL_STATES.map((s) => (
              <button key={s} className={`toggle ${c.accepts.includes(s) ? "on" : ""}`} onClick={() => toggleState(i, s)}>
                {STATE_LABEL[s]}
              </button>
            ))}
          </div>
        </div>
      ))}
      <div>
        <button
          className="btn sm"
          onClick={() => setCols((cs) => [...cs, { id: `col_${Date.now().toString(36)}`, name: "New column", order: cs.length, accepts: [], wip_limit: null, color: "neutral", hidden: false }])}
        >
          + Column
        </button>
      </div>
    </Modal>
  );
}

export function SettingsModal({
  settings,
  hooksInstalled,
  hooksPort,
  onClose,
  onSave,
  onStopAll,
  onHooks,
}: {
  settings: Settings;
  hooksInstalled: boolean;
  hooksPort: number;
  onClose: () => void;
  onSave: (s: Settings) => void;
  onStopAll: () => void;
  onHooks: (install: boolean) => void;
}) {
  const [s, setS] = useState<Settings>({ ...settings });
  const [paths, setPaths] = useState<Record<string, string> | null>(null);
  useEffect(() => {
    api.paths().then(setPaths).catch(() => {});
  }, []);
  const num = (k: keyof Settings) => (e: React.ChangeEvent<HTMLInputElement>) => setS({ ...s, [k]: Number(e.target.value) });
  return (
    <Modal
      title="Settings"
      onClose={onClose}
      footer={
        <>
          <button className="btn danger" onClick={onStopAll} title="Stop every background job kari started">
            Stop all kari jobs
          </button>
          <div className="spacer" />
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" onClick={() => onSave(s)}>
            Save
          </button>
        </>
      }
    >
      <div className="grid2">
        <div className="field">
          <label>History window (days)</label>
          <input type="number" value={s.history_days} onChange={num("history_days")} />
        </div>
        <div className="field">
          <label>Done after inactivity (days)</label>
          <input type="number" value={s.done_after_days} onChange={num("done_after_days")} />
        </div>
        <div className="field">
          <label>Stale after inactivity (days)</label>
          <input type="number" value={s.stale_after_days} onChange={num("stale_after_days")} />
        </div>
        <div className="field">
          <label>Max parallel background jobs</label>
          <input type="number" value={s.max_parallel_bg} onChange={num("max_parallel_bg")} />
        </div>
        <div className="field">
          <label>Terminal for Jump in</label>
          <select value={s.terminal_app} onChange={(e) => setS({ ...s, terminal_app: e.target.value })}>
            <option value="iTerm">iTerm2</option>
            <option value="Terminal">Terminal.app</option>
            <option value="Ghostty">Ghostty</option>
          </select>
        </div>
        <div className="field">
          <label>Default model for runs</label>
          <select value={s.default_run_model} onChange={(e) => setS({ ...s, default_run_model: e.target.value })}>
            {RUN_MODELS.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </div>
        <div className="field">
          <label>Default permission mode for unattended runs</label>
          <select value={s.default_permission_mode} onChange={(e) => setS({ ...s, default_permission_mode: e.target.value })}>
            {["auto", "acceptEdits", "bypassPermissions", "plan", "default"].map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </div>
      </div>
      <div className="section">
        <h5>Live hooks</h5>
        <div className="hint">
          Claude Code can tell kari the moment a session starts, stops, or waits for a permission. kari adds a small relay script to <code>~/.claude/settings.json</code> and keeps a backup. The relay
          never blocks a session when kari is closed.
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 8 }}>
          <span className={`pill ${hooksInstalled ? "on" : ""}`}>
            <span className="dot" />
            {hooksInstalled ? `hooks installed · port ${hooksPort}` : "hooks not installed"}
          </span>
          {hooksInstalled ? (
            <button className="btn sm" onClick={() => onHooks(false)}>
              Remove hooks
            </button>
          ) : (
            <button className="btn primary sm" onClick={() => onHooks(true)}>
              Install hooks
            </button>
          )}
        </div>
      </div>
      <div className="section">
        <h5>Summaries</h5>
        <div className="hint">kari asks Haiku for a two-sentence narrative after a turn ends. Calls are capped per hour and skip sessions older than the recent window.</div>
        <div className="grid2" style={{ marginTop: 8 }}>
          <label className="field inline">
            <input type="checkbox" checked={s.summaries_enabled} onChange={(e) => setS({ ...s, summaries_enabled: e.target.checked })} />
            <span>Summaries on</span>
          </label>
          <div className="field">
            <label>Model</label>
            <input value={s.summary_model} onChange={(e) => setS({ ...s, summary_model: e.target.value })} />
          </div>
          <div className="field">
            <label>Max calls per hour</label>
            <input type="number" value={s.summaries_per_hour} onChange={num("summaries_per_hour")} />
          </div>
          <div className="field">
            <label>Only sessions active in the last (hours)</label>
            <input type="number" value={s.summary_recent_hours} onChange={num("summary_recent_hours")} />
          </div>
        </div>
      </div>
      <div className="section">
        <h5>Scheduling</h5>
        <div className="hint">
          kari offers a plan when quota would expire unused. It never starts a card that is not marked "May run unattended". The planner keeps a reserve free during
          working hours and never fills a window past the ceiling.
        </div>
        <label className="field inline" style={{ marginTop: 8 }}>
          <input type="checkbox" checked={s.proposals_enabled} onChange={(e) => setS({ ...s, proposals_enabled: e.target.checked })} />
          <span>Offer plans</span>
        </label>
        <div className="grid2" style={{ marginTop: 8 }}>
          <div className="field">
            <label>Weekly window unused above (percent)</label>
            <input type="number" value={s.weekly_unused_pct} onChange={num("weekly_unused_pct")} />
          </div>
          <div className="field">
            <label>and resets within (hours)</label>
            <input type="number" value={s.weekly_hours_before_reset} onChange={num("weekly_hours_before_reset")} />
          </div>
          <div className="field">
            <label>5-hour window below (percent)</label>
            <input type="number" value={s.five_hour_idle_pct} onChange={num("five_hour_idle_pct")} />
          </div>
          <div className="field">
            <label>and nobody worked for (minutes)</label>
            <input type="number" value={s.idle_minutes} onChange={num("idle_minutes")} />
          </div>
          <div className="field">
            <label>Working hours start</label>
            <input type="number" min={0} max={23} value={s.working_hours_start} onChange={num("working_hours_start")} />
          </div>
          <div className="field">
            <label>Working hours end</label>
            <input type="number" min={0} max={23} value={s.working_hours_end} onChange={num("working_hours_end")} />
          </div>
          <div className="field">
            <label>Keep free in working hours (percent)</label>
            <input type="number" value={s.working_hours_reserve_pct} onChange={num("working_hours_reserve_pct")} />
          </div>
          <div className="field">
            <label>Never fill past (percent)</label>
            <input type="number" value={s.fill_ceiling_pct} onChange={num("fill_ceiling_pct")} />
          </div>
        </div>
      </div>
      <div className="section">
        <h5>Autopilot</h5>
        <div className="hint">
          With autopilot on, a weekly-reset plan starts by itself. kari still sends a notice and the plan panel keeps a Stop button. Only cards marked "May run
          unattended" are eligible, and the parallel cap holds.
        </div>
        <div className="grid2" style={{ marginTop: 8 }}>
          <label className="field inline">
            <input type="checkbox" checked={s.autopilot} onChange={(e) => setS({ ...s, autopilot: e.target.checked })} />
            <span>Start weekly-reset plans without asking</span>
          </label>
          <div className="field">
            <label>Jobs autopilot may start at once</label>
            <input type="number" min={1} value={s.autopilot_max_jobs} onChange={num("autopilot_max_jobs")} />
          </div>
          <label className="field inline">
            <input type="checkbox" checked={s.prefer_herdr} onChange={(e) => setS({ ...s, prefer_herdr: e.target.checked })} />
            <span>Open new sessions in a herdr pane</span>
          </label>
          <div className="field">
            <label>Warn when weekly unused is above (percent)</label>
            <input type="number" value={s.weekly_warn_unused_pct} onChange={num("weekly_warn_unused_pct")} />
          </div>
        </div>
      </div>
      <div className="section">
        <h5>Quota tracking</h5>
        <div className="hint">
          kari reads the 5-hour and 7-day windows from the Claude Code status line. Run the installer once. It wraps your current status line command and keeps a backup.
        </div>
        <pre className="code">scripts/install-statusline.sh</pre>
        <label className="field inline" style={{ marginTop: 8 }}>
          <input type="checkbox" checked={s.usage_endpoint_enabled} onChange={(e) => setS({ ...s, usage_endpoint_enabled: e.target.checked })} />
          <span>Ask the usage endpoint when no session refreshed the status line for 5 minutes</span>
        </label>
        <div className="hint">
          The endpoint is undocumented. kari reads the Claude Code login token from the keychain and asks at most once every 3 minutes. macOS shows a keychain prompt the first time.
        </div>
        {paths && (
          <div className="hint">
            Samples land in <code>{paths.rate_limits}</code>. Database: <code>{paths.db}</code>. kari {paths.version}.
          </div>
        )}
      </div>
    </Modal>
  );
}
