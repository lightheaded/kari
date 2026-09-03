import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import type { CloseGuard } from "../dirty";
import { useCloseGuard } from "../dirty";
import type { Column, DerivedState, NewTask, Settings } from "../types";
import { ALL_STATES, RUN_MODELS, STATE_HELP, STATE_LABEL } from "../types";
import { noAutoCorrect } from "../util";

/** The bar that a first Escape shows on a form with unsaved input. */
export function UnsavedBar({ guard, text, extra }: { guard: CloseGuard; text: string; extra?: React.ReactNode }) {
  if (!guard.asking) return null;
  return (
    <div className="unsaved" role="alert">
      <span>{text} Press Escape again to discard.</span>
      <div className="spacer" />
      {extra}
      <button className="btn sm" onClick={guard.keep} autoFocus>
        Keep editing
      </button>
      <button className="btn danger sm" onClick={guard.discard}>
        Discard
      </button>
    </div>
  );
}

function Modal({
  title,
  children,
  footer,
  onClose,
  guard,
  unsavedText,
}: {
  title: string;
  children: React.ReactNode;
  footer?: React.ReactNode;
  onClose: () => void;
  guard?: CloseGuard;
  unsavedText?: string;
}) {
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
        {guard && <UnsavedBar guard={guard} text={unsavedText ?? "This form holds unsaved input."} />}
        <div className="body" onInput={guard?.asking ? guard.keep : undefined}>
          {children}
        </div>
        {footer && <footer>{footer}</footer>}
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ project combobox

/**
 * A text field with a filtered list under it. The value is the directory. Typing
 * filters known projects by name or path. A path that is not in the list is
 * used as typed, so "Other path" needs no extra field.
 */
export function ProjectCombo({
  projects,
  value,
  onChange,
  autoFocus,
}: {
  projects: [string, string][];
  value: string;
  onChange: (cwd: string) => void;
  autoFocus?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [hi, setHi] = useState(0);
  const q = value.trim().toLowerCase();
  const matches = useMemo(() => {
    const exact = projects.some(([c]) => c === value);
    const list = exact || !q ? projects : projects.filter(([c, n]) => n.toLowerCase().includes(q) || c.toLowerCase().includes(q));
    return list.slice(0, 12);
  }, [projects, q, value]);
  const known = projects.find(([c]) => c === value);
  const listOpen = open && matches.length > 0;
  const pick = (cwd: string) => {
    onChange(cwd);
    setOpen(false);
  };
  return (
    <div className="combo">
      <input
        {...noAutoCorrect}
        autoFocus={autoFocus}
        value={value}
        placeholder="Type to search projects, or paste a path"
        role="combobox"
        aria-expanded={listOpen}
        aria-autocomplete="list"
        onChange={(e) => {
          onChange(e.target.value);
          setOpen(true);
          setHi(0);
        }}
        onFocus={() => setOpen(true)}
        onClick={() => setOpen(true)}
        onBlur={() => window.setTimeout(() => setOpen(false), 120)}
        onKeyDown={(e) => {
          if (!listOpen) {
            if (e.key === "ArrowDown") setOpen(true);
            return;
          }
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setHi((h) => Math.min(matches.length - 1, h + 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setHi((h) => Math.max(0, h - 1));
          } else if (e.key === "Enter") {
            e.preventDefault();
            pick(matches[hi][0]);
          } else if (e.key === "Escape") {
            // Close the list only. The modal stays open.
            e.preventDefault();
            e.stopPropagation();
            setOpen(false);
          }
        }}
      />
      <div className="hint">
        {known
          ? known[1]
          : matches.length > 0 && q
            ? `${matches.length} ${matches.length === 1 ? "project matches" : "projects match"}. Enter picks the highlighted one.`
            : value.startsWith("/")
              ? "A new path. kari adds it as a project."
              : value
                ? "No known project matches. Paste an absolute path."
                : " "}
      </div>
      {listOpen && (
        <ul className="combo-list" role="listbox">
          {matches.map(([c, n], i) => (
            <li
              key={c}
              role="option"
              aria-selected={i === hi}
              className={i === hi ? "hi" : ""}
              onMouseEnter={() => setHi(i)}
              onMouseDown={(e) => {
                e.preventDefault();
                pick(c);
              }}
            >
              <b>{n}</b>
              <span>{c}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

// ------------------------------------------------------------------ new task

interface Draft {
  title: string;
  cwd: string;
  prompt: string;
  autoRun: boolean;
  priority: number;
  notes: string;
  model: string;
}

const DRAFT_KEY = "kari.newTaskDraft";

function emptyDraft(cwd: string): Draft {
  return { title: "", cwd, prompt: "", autoRun: false, priority: 0, notes: "", model: "" };
}

function draftDirty(d: Draft): boolean {
  return !!(d.title.trim() || d.prompt.trim() || d.notes.trim() || d.autoRun || d.priority !== 0 || d.model);
}

function loadDraft(): Draft | null {
  try {
    const t = localStorage.getItem(DRAFT_KEY);
    if (!t) return null;
    const d = JSON.parse(t) as Partial<Draft>;
    if (typeof d.title !== "string") return null;
    return { ...emptyDraft(""), ...d };
  } catch {
    return null;
  }
}

function saveDraft(d: Draft | null) {
  try {
    if (d) localStorage.setItem(DRAFT_KEY, JSON.stringify(d));
    else localStorage.removeItem(DRAFT_KEY);
  } catch {
    // Storage can be off. The form still works, it just forgets on a restart.
  }
}

export function AddTaskModal({
  projects,
  onClose,
  onSubmit,
}: {
  projects: [string, string][];
  onClose: () => void;
  /** Resolves true when the task was added. The draft is then cleared. */
  onSubmit: (t: NewTask) => Promise<boolean>;
}) {
  // The draft lives in localStorage while the form is open, so a reload or a
  // restart of the app (tauri dev rebuilds on every Rust save) loses nothing.
  const [restored] = useState(() => {
    const d = loadDraft();
    return d && draftDirty(d) ? d : null;
  });
  const [d, setD] = useState<Draft>(() => restored ?? emptyDraft(projects[0]?.[0] ?? ""));
  const [showRestored, setShowRestored] = useState(!!restored);
  const dirty = draftDirty(d);
  useEffect(() => {
    saveDraft(dirty ? d : null);
  }, [d, dirty]);

  const guard = useCloseGuard(dirty, () => {
    saveDraft(null);
    onClose();
  });
  const set = <K extends keyof Draft>(k: K, v: Draft[K]) => setD((x) => ({ ...x, [k]: v }));
  const [busy, setBusy] = useState(false);
  const submit = async () => {
    if (!d.title.trim() || busy) return;
    setBusy(true);
    const ok = await onSubmit({
      title: d.title.trim(),
      project_cwd: d.cwd.trim() || null,
      run_prompt: d.prompt.trim() || null,
      auto_run: d.autoRun,
      priority: d.priority,
      notes: d.notes.trim() || null,
      model: d.model || null,
    });
    setBusy(false);
    if (ok) saveDraft(null);
  };

  return (
    <Modal
      title="New task"
      onClose={guard.requestClose}
      guard={guard}
      unsavedText="This task is not saved yet."
      footer={
        <>
          <button className="btn" onClick={guard.requestClose}>
            Cancel
          </button>
          <button className="btn primary" disabled={!d.title.trim() || busy} onClick={submit}>
            Add
          </button>
        </>
      }
    >
      {showRestored && (
        <div className="hint restored">
          Restored the task you typed before the app restarted.
          <button
            className="linkish"
            onClick={() => {
              setD(emptyDraft(projects[0]?.[0] ?? ""));
              setShowRestored(false);
            }}
          >
            Start fresh
          </button>
        </div>
      )}
      <div className="field">
        <label>Title</label>
        <input
          {...noAutoCorrect}
          autoFocus
          value={d.title}
          onChange={(e) => set("title", e.target.value)}
          placeholder="What needs to happen"
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) submit();
          }}
        />
      </div>
      <div className="field">
        <label>Project directory</label>
        <ProjectCombo projects={projects} value={d.cwd} onChange={(v) => set("cwd", v)} />
      </div>
      <div className="field">
        <label>Run prompt (what Claude gets when the task starts)</label>
        <textarea {...noAutoCorrect} value={d.prompt} onChange={(e) => set("prompt", e.target.value)} placeholder="Leave empty to use the title" />
      </div>
      <div className="grid2">
        <div className="field">
          <label>Priority</label>
          <input type="number" value={d.priority} onChange={(e) => set("priority", Number(e.target.value))} title="Higher runs first. A drag inside a column also sets this." />
        </div>
        <div className="field">
          <label>Model (optional)</label>
          <select value={d.model} onChange={(e) => set("model", e.target.value)}>
            {RUN_MODELS.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </div>
      </div>
      <label className="field inline">
        <input type="checkbox" checked={d.autoRun} onChange={(e) => set("autoRun", e.target.checked)} />
        <span>May run unattended</span>
      </label>
      <div className="field">
        <label>Notes</label>
        <textarea {...noAutoCorrect} value={d.notes} onChange={(e) => set("notes", e.target.value)} />
      </div>
      <div className="hint">Cmd+Enter adds the task. The draft is kept until you add it or discard it.</div>
    </Modal>
  );
}

// ------------------------------------------------------------------ columns

const COLORS = ["neutral", "green", "amber", "rust", "slate"];

export function ColumnsModal({ columns, onClose, onSave, onReset }: { columns: Column[]; onClose: () => void; onSave: (c: Column[]) => void; onReset: () => void }) {
  const initial = useMemo(() => [...columns].sort((a, b) => a.order - b.order).map((c) => ({ ...c })), [columns]);
  const [cols, setCols] = useState<Column[]>(initial);
  const dirty = JSON.stringify(cols) !== JSON.stringify(initial);
  const guard = useCloseGuard(dirty, onClose);
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
      onClose={guard.requestClose}
      guard={guard}
      unsavedText="The column changes are not saved."
      footer={
        <>
          <button className="btn ghost" onClick={onReset}>
            Reset to defaults
          </button>
          <div className="spacer" />
          <button className="btn" onClick={guard.requestClose}>
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
        accepts Stale. Hover a state for what it means.
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
          <input {...noAutoCorrect} type="text" value={c.name} onChange={(e) => update(i, { name: e.target.value })} />
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
              <button key={s} className={`toggle ${c.accepts.includes(s) ? "on" : ""}`} onClick={() => toggleState(i, s)} title={STATE_HELP[s]}>
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

// ------------------------------------------------------------------ settings

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
  const dirty = JSON.stringify(s) !== JSON.stringify(settings);
  const guard = useCloseGuard(dirty, onClose);
  useEffect(() => {
    api.paths().then(setPaths).catch(() => {});
  }, []);
  const num = (k: keyof Settings) => (e: React.ChangeEvent<HTMLInputElement>) => setS({ ...s, [k]: Number(e.target.value) });
  return (
    <Modal
      title="Settings"
      onClose={guard.requestClose}
      guard={guard}
      unsavedText="The settings are not saved."
      footer={
        <>
          <button className="btn danger" onClick={onStopAll} title="Stop every background job kari started">
            Stop all kari jobs
          </button>
          <div className="spacer" />
          <button className="btn" onClick={guard.requestClose}>
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
            <input {...noAutoCorrect} value={s.summary_model} onChange={(e) => setS({ ...s, summary_model: e.target.value })} />
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
          kari reads the 5-hour and 7-day windows from the Claude Code status line. Run the installer once. It wraps your current status line command and keeps a backup. The
          status line only refreshes while a session runs. Without one, the sample ages and the quota bar shows "stale".
        </div>
        <pre className="code">scripts/install-statusline.sh</pre>
        <label className="field inline" style={{ marginTop: 8 }}>
          <input type="checkbox" checked={s.usage_endpoint_enabled} onChange={(e) => setS({ ...s, usage_endpoint_enabled: e.target.checked })} />
          <span>Refresh the quota by yourself when no session updated it for 5 minutes</span>
        </label>
        <div className="hint">
          This asks the usage endpoint of the Claude account, at most once every 3 minutes, also when no process runs. The endpoint is undocumented. kari reads the Claude Code
          login token from the keychain, and macOS shows a keychain prompt the first time. A click on "stale" in the quota bar asks once, with this setting on or off.
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
