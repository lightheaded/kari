import { useEffect, useState } from "react";
import { api } from "../api";
import type { Column, HubCard, JobLogEntry, NodeStatus, Project, Settings } from "../types";
import { RUN_MODELS, STATE_LABEL } from "../types";
import { clock, fmtM, fmtPct, noAutoFill, proseField, relTime, shortId, weighted } from "../util";
import { useAutoGrow } from "../hooks";
import { useCloseGuard } from "../dirty";
import { UnsavedBar } from "./Modals";
import { ProjectPicker, type PickerItem } from "./ProjectPicker";

interface Props {
  view: HubCard;
  columns: Column[];
  settings: Settings | null;
  /** Every node on the board. A task card can move to another one. */
  nodes?: NodeStatus[];
  /** Projects of this card's node, from the board. Used until the node answers. */
  projects?: Project[];
  /** Show which node the card comes from. Set when the board has more than one node. */
  showNode?: boolean;
  /** The node does not answer. Every action is off until it comes back. */
  offline?: boolean;
  /** A phone: no terminal here, so Jump in gives way to the command to run elsewhere. */
  mobile?: boolean;
  onClose: () => void;
  onAction: (fn: () => Promise<unknown>, ok?: string) => Promise<void>;
  /** The card moved to another node, where it has a new id. Select it there. */
  onMoved?: (nodeId: string, cardId: string) => void;
}

const MODES = ["", "bypassPermissions", "acceptEdits", "auto", "plan", "default"];

/** The picker value that asks for a typed path. No project directory uses it. */
const OTHER = "__custom";

export function Drawer({
  view,
  columns,
  settings,
  nodes = [],
  projects = [],
  showNode,
  offline,
  mobile,
  onClose,
  onAction,
  onMoved,
}: Props) {
  const c = view.card;
  const node = view.node_id;
  const s = view.session;
  const [title, setTitle] = useState(c.title ?? "");
  const [prompt, setPrompt] = useState(c.run_prompt ?? "");
  const [notes, setNotes] = useState(c.notes ?? "");
  const [priority, setPriority] = useState(c.priority);
  const [autoRun, setAutoRun] = useState(c.auto_run);
  const [mode, setMode] = useState(c.permission_mode ?? "");
  const [model, setModel] = useState(c.model ?? "");
  const [startPrompt, setStartPrompt] = useState("");
  const [log, setLog] = useState<JobLogEntry[]>([]);
  /** The project directory of the card. OTHER means "use the typed path". */
  const [cwd, setCwd] = useState(c.project_cwd ?? "");
  const [customCwd, setCustomCwd] = useState("");
  /** Every project one node knows, as that node answered. */
  const [nodeProjects, setNodeProjects] = useState<{ node: string; list: Project[] } | null>(null);
  /** The node the user picked in the move field, and the card it was picked
   *  for. Reading the card back means another card resets the field, and a
   *  poll of the board never does. */
  const [picked, setPicked] = useState<{ card: string; node: string } | null>(null);
  const moveTo = picked?.card === c.id ? picked.node : node;
  const promptGrow = useAutoGrow("drawer.prompt", prompt, 90, 520);
  const notesGrow = useAutoGrow("drawer.notes", notes, 68, 400);
  const startGrow = useAutoGrow("drawer.startPrompt", startPrompt);

  useEffect(() => {
    setTitle(c.title ?? "");
    setPrompt(c.run_prompt ?? "");
    setNotes(c.notes ?? "");
    setPriority(c.priority);
    setAutoRun(c.auto_run);
    setMode(c.permission_mode ?? "");
    setModel(c.model ?? "");
    setCwd(c.project_cwd ?? "");
    setCustomCwd("");
  }, [
    c.id,
    c.updated_at,
    c.title,
    c.run_prompt,
    c.notes,
    c.priority,
    c.auto_run,
    c.permission_mode,
    c.model,
    c.project_cwd,
  ]);

  // Ask the node for every project it knows. The board only names the projects
  // that already have a card, so without this a first move has nothing to pick.
  useEffect(() => {
    let live = true;
    api
      .projects(node)
      .then((list) => live && list.length > 0 && setNodeProjects({ node, list }))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [node]);

  useEffect(() => {
    let live = true;
    api
      .jobLog(node, c.id)
      .then((l) => live && setLog(l))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [node, c.id, c.updated_at]);

  // The projects to choose from, and the path the card gets on save. The board
  // names the projects that have a card, until the node lists them all.
  const projectList = nodeProjects?.node === node ? nodeProjects.list : projects;
  const dir = cwd === OTHER ? customCwd.trim() : cwd;
  const projectItems: PickerItem[] = [
    // A card can hold a path that no other card on this node holds, such as a
    // typed one. Keep it in the list, or the picker reads as if it were empty.
    ...(cwd && cwd !== OTHER && !projectList.some((p) => p.cwd === cwd)
      ? [{ value: cwd, label: cwd.split("/").filter(Boolean).pop() ?? cwd, hint: cwd }]
      : []),
    ...projectList.map((p) => ({ value: p.cwd, label: p.name, hint: p.cwd })),
    { value: OTHER, label: "Other path…" },
  ];

  const dirty =
    title !== (c.title ?? "") ||
    prompt !== (c.run_prompt ?? "") ||
    notes !== (c.notes ?? "") ||
    priority !== c.priority ||
    autoRun !== c.auto_run ||
    mode !== (c.permission_mode ?? "") ||
    model !== (c.model ?? "") ||
    dir !== (c.project_cwd ?? "") ||
    startPrompt.trim() !== "";

  // An edit in the drawer is worth more than a stray Escape. The first one asks.
  const guard = useCloseGuard(dirty, onClose);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") guard.requestClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [guard]);

  const save = () =>
    onAction(
      () =>
        api.patchCard(node, c.id, {
          title: c.kind === "task" || title !== (c.title ?? "") ? title : undefined,
          project_cwd: dir !== (c.project_cwd ?? "") ? dir : undefined,
          run_prompt: prompt,
          notes,
          priority,
          auto_run: autoRun,
          permission_mode: mode,
          model,
        }),
      "Saved",
    );

  // Only a task card that never ran can move. A session card follows a
  // transcript that stays on its own machine.
  const canMoveNode = nodes.length > 1 && c.kind === "task" && !c.session_id;
  const moveTarget = nodes.find((n) => n.id === moveTo);
  const move = () =>
    onAction(async () => {
      const moved = await api.moveCardToNode(node, c.id, moveTo);
      onMoved?.(moveTo, moved.id);
      return `Moved to ${moveTarget?.name ?? moveTo}`;
    }, "Moved");

  const doneCol = columns.find((k) => k.accepts.includes("done"));
  const canStart = !!(c.project_cwd ?? s?.cwd) && !(view.bg_job && view.bg_job.state === "working");
  const q = s?.pending_tools.filter((t) => t.name === "AskUserQuestion") ?? [];

  return (
    <aside className="drawer" onInput={guard.asking ? guard.keep : undefined}>
      <button className="btn ghost sm close" onClick={guard.requestClose} aria-label="Close">
        ✕
      </button>
      <UnsavedBar guard={guard} text="This card has unsaved edits." />
      <header>
        <h2>{view.title}</h2>
        <div className="hint">
          {showNode ? `${view.node_name} · ` : ""}
          {STATE_LABEL[view.state]} · {view.reason}
          {view.locked ? " · manual placement" : ""}
        </div>
        {offline && <div className="hint offline-note">This node is offline. Actions return when it reconnects.</div>}
        <div className="actions">
          {!mobile && (
            <button className="btn primary sm" disabled={offline} onClick={() => onAction(() => api.jumpIn(node, c.id), "Opened")}>
              Jump in
            </button>
          )}
          {canStart && (
            <button
              className="btn sm"
              disabled={offline}
              title={c.session_id ? "Continue this session as a background job" : "Start as a background job"}
              onClick={() => onAction(() => api.startCard(node, c.id, startPrompt || undefined), "Started in background")}
            >
              ▶ {c.session_id ? "Continue in bg" : "Start in bg"}
            </button>
          )}
          {view.bg_job?.state === "working" && (
            <button className="btn danger sm" disabled={offline} onClick={() => onAction(() => api.stopCard(node, c.id), "Stopped")}>
              ■ Stop job
            </button>
          )}
          {doneCol && view.state !== "done" && (
            <button className="btn sm" disabled={offline} onClick={() => onAction(() => api.moveCard(node, c.id, doneCol.id), "Marked done")}>
              ✓ Done
            </button>
          )}
          {c.session_id && s && s.turns > 0 && (
            <button
              className="btn ghost sm"
              disabled={offline}
              title="Ask Haiku for a fresh summary now"
              onClick={() => onAction(() => api.summarizeCard(node, c.id), "Summary updated")}
            >
              ✦ Summarize
            </button>
          )}
          <button className="btn ghost sm" disabled={offline} onClick={() => onAction(() => api.patchCard(node, c.id, { archived: true }), "Archived").then(onClose)}>
            Archive
          </button>
          {c.kind === "task" && (
            <button className="btn ghost sm" disabled={offline} onClick={() => onAction(() => api.deleteCard(node, c.id), "Deleted").then(onClose)}>
              Delete
            </button>
          )}
        </div>
      </header>
      <div className="body">
        {view.permission && (
          <div className="section">
            <h5>Held permission prompt</h5>
            <div className="quote">
              {view.permission.tool_name}
              {"\n"}
              {typeof view.permission.tool_input === "string" ? view.permission.tool_input : JSON.stringify(view.permission.tool_input, null, 2)}
            </div>
            {!offline && (
              <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
                <button className="btn primary sm" onClick={() => onAction(() => api.answerPermission(node, view.permission!.id, "allow"), "Allowed")}>
                  Allow
                </button>
                <button className="btn danger sm" onClick={() => onAction(() => api.answerPermission(node, view.permission!.id, "deny"), "Denied")}>
                  Deny
                </button>
              </div>
            )}
          </div>
        )}

        {q.length > 0 && (
          <div className="section">
            <h5>Open questions</h5>
            {q.flatMap((t) => t.questions).map((qq, i) => (
              <div key={i} className="quote" style={{ marginBottom: 6 }}>
                {qq.question}
                {qq.options.length > 0 && (
                  <ul style={{ margin: "6px 0 0", paddingLeft: 18 }}>
                    {qq.options.map((o) => (
                      <li key={o}>{o}</li>
                    ))}
                  </ul>
                )}
              </div>
            ))}
          </div>
        )}

        {view.summary && (
          <div className="section">
            <h5>
              Summary <span className="soft">· {view.summary.source} · {relTime(view.summary.generated_at)} ago · judged {STATE_LABEL[view.summary.judged_state]} ({Math.round(view.summary.confidence * 100)}%)</span>
            </h5>
            <div className="narrative-block">{view.summary.narrative}</div>
            {view.summary.next_step && (
              <div className="hint" style={{ marginTop: 6 }}>
                Next: {view.summary.next_step}
              </div>
            )}
            {view.summary.open_questions.length > 0 && q.length === 0 && (
              <ul className="soft-list">
                {view.summary.open_questions.map((x) => (
                  <li key={x}>{x}</li>
                ))}
              </ul>
            )}
          </div>
        )}

        <dl className="kv">
          <dt>Project</dt>
          <dd>
            {view.project_name ?? "—"}
            {c.project_cwd || s?.cwd ? <div className="hint">{c.project_cwd ?? s?.cwd}</div> : null}
          </dd>
          {c.session_id && (
            <>
              <dt>Session</dt>
              <dd>
                <code>{shortId(c.session_id)}</code>
                {s?.git_branch ? ` · ${s.git_branch}` : ""}
                {s?.version ? ` · v${s.version}` : ""}
              </dd>
            </>
          )}
          {view.live && (
            <>
              <dt>Process</dt>
              <dd>
                pid {view.live.pid} · {view.live.status ?? "?"} · {view.live.name ?? ""}
              </dd>
            </>
          )}
          {view.herdr && (
            <>
              <dt>herdr</dt>
              <dd>
                {view.herdr.workspace_label ?? view.herdr.workspace_id} · pane {view.herdr.pane_id} · {view.herdr.agent_status}
              </dd>
            </>
          )}
          {view.hooks && (
            <>
              <dt>Hooks</dt>
              <dd>
                {view.hooks.last_event} {relTime(view.hooks.last_at)} ago · {view.hooks.events_seen} events
                {view.hooks.permission_pending_since ? ` · waits for permission: ${view.hooks.permission_message ?? ""}` : ""}
              </dd>
            </>
          )}
          {view.estimate && (
            <>
              <dt>Estimate</dt>
              <dd>
                {fmtPct(view.estimate.pct_five_hour)} of the 5-hour window · {fmtM(view.estimate.weighted_tokens)} weighted tokens
                <div className="hint">
                  band {fmtPct(view.estimate.pct_low)} to {fmtPct(view.estimate.pct_high)} · from {view.estimate.source}
                  {view.estimate.sessions > 0 ? ` (${view.estimate.sessions} past sessions)` : ""}
                </div>
              </dd>
            </>
          )}
          {view.bg_job && (
            <>
              <dt>Background</dt>
              <dd>
                job {view.bg_job.id} · {view.bg_job.state}
                {view.bg_job.waiting_for ? ` · waits: ${view.bg_job.waiting_for}` : ""}
              </dd>
            </>
          )}
          {s && (
            <>
              <dt>Activity</dt>
              <dd>
                {s.turns} prompts · first {clock(s.first_at)} · last {clock(view.last_activity_at)} ({relTime(view.last_activity_at)} ago)
              </dd>
              <dt>Tokens</dt>
              <dd>
                {fmtM(weighted(s.tokens))} weighted · in {fmtM(s.tokens.input)} · out {fmtM(s.tokens.output)} · cache r {fmtM(s.tokens.cache_read)} / w{" "}
                {fmtM(s.tokens.cache_write)} · {s.tokens.messages} replies
              </dd>
              {s.models.length > 0 && (
                <>
                  <dt>Models</dt>
                  <dd>{s.models.join(", ")}</dd>
                </>
              )}
              {s.pr_links.length > 0 && (
                <>
                  <dt>PRs</dt>
                  <dd>
                    {s.pr_links.map((u) => (
                      <div key={u}>
                        {/^https?:\/\//.test(u) ? (
                          <a href={u} target="_blank" rel="noreferrer">
                            {u}
                          </a>
                        ) : (
                          u
                        )}
                      </div>
                    ))}
                  </dd>
                </>
              )}
            </>
          )}
        </dl>

        {log.length > 0 && (
          <div className="section">
            <h5>
              Run log <span className="soft">· {log.length} entries</span>
            </h5>
            <ul className="runlog">
              {log.map((l, i) => (
                <li key={`${l.at}-${i}`}>
                  <span className="when">{clock(l.at)}</span>
                  <span className={`st ${l.state ?? ""}`}>{l.state ?? "?"}</span>
                  <span className="det">
                    {l.detail ?? ""}
                    {l.job_id ? ` · ${l.job_id.slice(0, 8)}` : ""}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        )}

        {mobile && c.session_id && (
          <div className="section">
            <h5>In a terminal</h5>
            <div className="hint">On {view.node_name}, in {c.project_cwd ?? s?.cwd ?? "the project"}:</div>
            <div className="quote">claude --resume {c.session_id}</div>
          </div>
        )}

        {s?.last_prompt && (
          <div className="section">
            <h5>Last prompt</h5>
            <div className="quote">{s.last_prompt}</div>
          </div>
        )}
        {s?.last_assistant_text && (
          <div className="section">
            <h5>Last reply</h5>
            <div className="quote">{s.last_assistant_text}</div>
          </div>
        )}
        {s?.first_prompt && s.first_prompt !== s.last_prompt && (
          <div className="section">
            <h5>First prompt</h5>
            <div className="quote">{s.first_prompt}</div>
          </div>
        )}

        <div className="section">
          <h5>Card</h5>
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            <div className="field">
              <label>{c.kind === "task" ? "Title" : "Title override"}</label>
              <input {...noAutoFill} value={title} onChange={(e) => setTitle(e.target.value)} placeholder={view.title} />
            </div>
            <div className="field">
              <label>Project directory{showNode ? ` on ${view.node_name}` : ""}</label>
              <ProjectPicker
                items={projectItems}
                value={cwd}
                allLabel={c.session_id ? "From the session" : "No project"}
                ariaLabel="Project directory"
                onChange={setCwd}
              />
              {cwd === OTHER && (
                <input
                  {...noAutoFill}
                  value={customCwd}
                  onChange={(e) => setCustomCwd(e.target.value)}
                  placeholder="/absolute/path"
                />
              )}
              <div className="hint">
                {c.session_id
                  ? `Empty uses the directory of the session${s?.cwd ? `: ${s.cwd}` : ""}.`
                  : "A run and Jump in both start here. The node refuses a path that is not a directory there."}
              </div>
            </div>
            <div className="field">
              <label>
                {c.session_id ? "Continue prompt (used by Start in bg and the scheduler)" : "Body (added under the title when the task runs)"}
              </label>
              <textarea
                {...proseField}
                {...promptGrow}
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                placeholder={
                  c.session_id
                    ? "Continue with the next step. Stop when done."
                    : "Detail, links, where to start. The title is always the first line, so it needs no repeating."
                }
              />
              {c.kind === "task" && (
                <div className="hint">
                  A run receives the title, a blank line, then this body.
                </div>
              )}
            </div>
            <div className="grid2">
              <div className="field">
                <label>Priority</label>
                <input
                  {...noAutoFill}
                  type="number"
                  value={priority}
                  onChange={(e) => setPriority(Number(e.target.value))}
                  title="0 means automatic order. Dragging the card on the board writes this number."
                />
              </div>
              <div className="field">
                <label>Model for runs</label>
                <select value={model} onChange={(e) => setModel(e.target.value)}>
                  {RUN_MODELS.map((m) => (
                    <option key={m.value} value={m.value}>
                      {m.label}
                    </option>
                  ))}
                </select>
              </div>
              <div className="field">
                <label>Permission mode</label>
                <select value={mode} onChange={(e) => setMode(e.target.value)}>
                  {MODES.map((m) => (
                    <option key={m} value={m}>
                      {m || `default (${settings?.default_permission_mode ?? "auto"})`}
                    </option>
                  ))}
                </select>
              </div>
            </div>
            <label className="field inline">
              <input type="checkbox" checked={autoRun} onChange={(e) => setAutoRun(e.target.checked)} />
              <span>May run unattended when quota is left over</span>
            </label>
            {canMoveNode && (
              <div className="field">
                <label>Node</label>
                <select value={moveTo} onChange={(e) => setPicked({ card: c.id, node: e.target.value })}>
                  {nodes.map((n) => (
                    <option key={n.id} value={n.id}>
                      {n.name}
                      {n.online ? "" : " (offline)"}
                    </option>
                  ))}
                </select>
                {moveTo !== node && (
                  <>
                    <div className="hint">
                      Each node keeps its own cards, so a move writes the card again on{" "}
                      {moveTarget?.name ?? moveTo} and gives it a new id. It keeps the project only when that
                      node holds one project of the same name.
                    </div>
                    <div style={{ marginTop: 6 }}>
                      <button
                        className="btn sm"
                        disabled={offline || dirty || moveTarget?.online === false}
                        title={dirty ? "Save the card first" : `Move this card to ${moveTarget?.name ?? moveTo}`}
                        onClick={move}
                      >
                        Move to {moveTarget?.name ?? moveTo}
                      </button>
                      {dirty && <span className="hint"> Save the card first.</span>}
                    </div>
                  </>
                )}
              </div>
            )}
            {nodes.length > 1 && !canMoveNode && (
              <div className="field">
                <label>Node</label>
                <div className="hint">
                  {view.node_name}. This card follows a session on that node, so it cannot move.
                </div>
              </div>
            )}
            <div className="field">
              <label>Notes</label>
              <textarea {...proseField} {...notesGrow} value={notes} onChange={(e) => setNotes(e.target.value)} />
            </div>
            <div style={{ display: "flex", gap: 8 }}>
              <button className="btn primary sm" disabled={!dirty || offline} onClick={save}>
                Save card
              </button>
            </div>
          </div>
        </div>

        {canStart && (
          <div className="section">
            <h5>One-off background prompt</h5>
            <div className="field">
              <textarea
                {...proseField}
                {...startGrow}
                value={startPrompt}
                onChange={(e) => setStartPrompt(e.target.value)}
                placeholder="Optional prompt for the next Start / Continue in bg. Empty uses the title and the body."
              />
            </div>
          </div>
        )}
      </div>
    </aside>
  );
}
