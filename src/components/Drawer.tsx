import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import type { Attachment, CardPatch, Column, HubCard, JobLogEntry, NodeStatus, Project, QuotaSample, ScheduleWhen, Settings } from "../types";
import { RUN_MODELS, STATE_LABEL } from "../types";
import { clearsBox, clock, fmtM, fmtPct, noAutoFill, proseField, relTime, schedulePreview, shortId, untilTime, weighted } from "../util";
import { useAutoGrow } from "../hooks";
import { useCloseGuard } from "../dirty";
import { UnsavedBar } from "./Modals";
import type { Act } from "../toasts";
import { ProjectPicker, type PickerItem } from "./ProjectPicker";
import { Markdown } from "./Markdown";
import { ConversationList, popOutConversation, useConversation } from "./Conversation";
import { AttachButton, AttachmentList, collectFiles } from "./Attachments";

interface Props {
  view: HubCard;
  columns: Column[];
  settings: Settings | null;
  /** Every node on the board. A task card can move to another one. */
  nodes?: NodeStatus[];
  /** Projects of this card's node, from the board. Used until the node answers. */
  projects?: Project[];
  /** The rate-limit sample of this card's node. It dates the cycles the
   *  schedule buttons offer. Without it the node cannot book a cycle. */
  quota?: QuotaSample | null;
  /** Show which node the card comes from. Set when the board has more than one node. */
  showNode?: boolean;
  /** The node does not answer. Every action is off until it comes back. */
  offline?: boolean;
  /** A phone: no terminal here, so Jump in gives way to the command to run elsewhere. */
  mobile?: boolean;
  onClose: () => void;
  onAction: Act;
  /** The card moved to another node, where it has a new id. Select it there. */
  onMoved?: (nodeId: string, cardId: string) => void;
}

const MODES = ["", "bypassPermissions", "acceptEdits", "auto", "plan", "default"];

/** The picker value that asks for a typed path. No project directory uses it. */
const OTHER = "__custom";

/** A text field that keeps its own draft and saves on blur.
 *  The saved value comes from the card; the draft follows it until the user
 *  types, so a board refresh never overwrites a half-typed note. */
function useSavedText(saved: string, save: (v: string) => void) {
  const [draft, setDraft] = useState(saved);
  const [base, setBase] = useState(saved);
  if (saved !== base) {
    // The card changed under us. Only a draft the user left alone follows it.
    setBase(saved);
    if (draft === base) setDraft(saved);
  }
  const flush = useCallback(() => {
    if (draft !== saved) save(draft);
  }, [draft, saved, save]);
  return { draft, setDraft, flush, dirty: draft !== saved };
}

/** The title of the card, and an input in its place on a click. */
function TitleEdit({
  title,
  saved,
  disabled,
  onSave,
}: {
  /** What the board shows. */
  title: string;
  /** The stored title, or the empty string for a session that has no override. */
  saved: string;
  disabled?: boolean;
  onSave: (title: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(saved || title);
  const ref = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    if (editing) ref.current?.select();
  }, [editing]);
  const commit = () => {
    setEditing(false);
    const t = draft.trim();
    if (t !== (saved || title) && (t || saved)) onSave(t);
  };
  if (!editing) {
    return (
      <h2
        className={`titleedit ${disabled ? "" : "can"}`}
        title={disabled ? undefined : "Click to rename"}
        onClick={() => {
          if (disabled) return;
          setDraft(saved || title);
          setEditing(true);
        }}
      >
        {title}
      </h2>
    );
  }
  return (
    <input
      {...noAutoFill}
      ref={ref}
      className="titleinput"
      value={draft}
      aria-label="Title"
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
        if (e.key === "Escape") {
          // Ours, not the drawer's: the edit ends and the drawer stays.
          e.stopPropagation();
          setEditing(false);
        }
      }}
    />
  );
}

export function Drawer({
  view,
  columns,
  settings,
  nodes = [],
  projects = [],
  quota,
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
  const picked = useMemo(() => ({ node, id: c.id }), [node, c.id]);

  /** Write one change to the card. No toast on success: the field shows it. */
  const patch = useCallback((p: CardPatch) => onAction(() => api.patchCard(node, c.id, p), undefined, undefined, picked), [onAction, node, c.id, picked]);

  const prompt = useSavedText(c.run_prompt ?? "", (v) => patch({ run_prompt: v }));
  const notes = useSavedText(c.notes ?? "", (v) => patch({ notes: v }));
  const [draft, setDraft] = useState("");
  /** The card whose schedule block is open, and the time typed into it. This
   *  holds the card id, exactly as the move field does, so another card closes
   *  the block and a poll of the board never does. */
  const [booking, setBooking] = useState<{ card: string; at: string } | null>(null);
  const [log, setLog] = useState<JobLogEntry[]>([]);
  /** The project directory of the card. OTHER means "use the typed path". */
  const [cwd, setCwd] = useState(c.project_cwd ?? "");
  const [customCwd, setCustomCwd] = useState("");
  /** Every project one node knows, as that node answered. */
  const [nodeProjects, setNodeProjects] = useState<{ node: string; list: Project[] } | null>(null);
  /** The node the user picked in the move field, and the card it was picked
   *  for. Reading the card back means another card resets the field, and a
   *  poll of the board never does. */
  const [pickedNode, setPickedNode] = useState<{ card: string; node: string } | null>(null);
  const moveTo = pickedNode?.card === c.id ? pickedNode.node : node;
  const promptGrow = useAutoGrow("drawer.prompt", prompt.draft, 34, 520);
  const notesGrow = useAutoGrow("drawer.notes", notes.draft, 34, 400);
  const draftGrow = useAutoGrow("drawer.compose", draft, 34, 300);

  /** What the node says the card holds. The board carries the list, and an
   *  upload answers with the file it wrote, so the strip is right before the
   *  next board arrives. */
  const [added, setAdded] = useState<{ card: string; items: Attachment[] } | null>(null);
  const attachments = added?.card === c.id ? added.items : (view.attachments ?? []);
  const [attachBusy, setAttachBusy] = useState(false);

  /** Whether the whole conversation is on show, or only the last exchange. */
  const [convOpen, setConvOpen] = useState(false);
  const { conv, busy: convBusy, err: convErr, load: loadConv } = useConversation(node, c.id, convOpen && !!c.session_id, view.last_activity_at);

  useEffect(() => {
    setCwd(c.project_cwd ?? "");
    setCustomCwd("");
  }, [c.id, c.project_cwd]);

  // A new card closes the conversation and empties the composer: the draft
  // was for the card before.
  useEffect(() => {
    setConvOpen(false);
    setDraft("");
    setAdded(null);
  }, [c.id]);

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
  const pickProject = (v: string) => {
    setCwd(v);
    if (v !== OTHER && v !== (c.project_cwd ?? "")) patch({ project_cwd: v });
  };
  const saveCustomCwd = () => {
    if (cwd === OTHER && dir && dir !== (c.project_cwd ?? "")) patch({ project_cwd: dir });
  };

  /** Save whatever text still waits for a blur. Runs before the drawer closes. */
  const flush = useCallback(() => {
    prompt.flush();
    notes.flush();
  }, [prompt, notes]);

  // A typed prompt is worth more than a stray Escape. The first one asks.
  const guard = useCloseGuard(draft.trim() !== "", onClose);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        flush();
        guard.requestClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [guard, flush]);

  const close = () => {
    flush();
    guard.requestClose();
  };

  // Only a task card that never ran can move. A session card follows a
  // transcript that stays on its own machine.
  const canMoveNode = nodes.length > 1 && c.kind === "task" && !c.session_id;
  const moveTarget = nodes.find((n) => n.id === moveTo);
  const move = () =>
    onAction(
      async () => {
        const moved = await api.moveCardToNode(node, c.id, moveTo);
        onMoved?.(moveTo, moved.id);
        return `Moved to ${moveTarget?.name ?? moveTo}`;
      },
      "Moved",
      undefined,
      picked,
    );

  const doneCol = columns.find((k) => k.accepts.includes("done"));
  const q = s?.pending_tools.filter((t) => t.name === "AskUserQuestion") ?? [];
  const bg = view.bg_job;
  const running = !!view.live?.alive;
  const jobBusy = !running && bg?.state === "working";
  const hasDir = !!(c.project_cwd ?? s?.cwd);

  // The run the user booked, and the times each choice would book. The node
  // resolves the real time from its own sample, so these labels are a preview.
  const booked = c.scheduled;
  const canBook = hasDir && bg?.state !== "working";
  const bookingOpen = booking?.card === c.id;
  const bookAt = bookingOpen ? booking.at : "";
  const preview = useMemo(() => schedulePreview(quota), [quota]);
  const cycles: { when: ScheduleWhen; label: string }[] = [
    { when: "next_reset", label: "When the window resets" },
    { when: "following_cycle", label: "The cycle after that" },
    { when: "weekly_reset", label: "When the week resets" },
  ];
  /** Book the run. The composer at the foot holds the one-off prompt, so its
   *  text goes with the booking and the box empties once the booking lands. */
  const book = (when: ScheduleWhen, at?: string) => {
    const text = draft.trim();
    return onAction(
      async () => {
        await api.scheduleCard(node, c.id, when, { at, prompt: text || undefined });
        setBooking(null);
      },
      "Run booked",
      { done: "Booking cancelled", run: () => api.cancelSchedule(node, c.id), label: "Cancel" },
      picked,
    ).then((sent) => {
      setDraft((d) => (clearsBox(d, text, sent) ? "" : d));
      return sent;
    });
  };

  /** Where the composer sends, in one line, and the button that says so. */
  const target = running
    ? { label: "Send", hint: `Goes into the running session (pid ${view.live!.pid}). An idle session answers at once; a busy one takes it after the current turn.` }
    : jobBusy
      ? { label: "Send", hint: "A background job works on this card. The prompt waits until it is reachable." }
      : c.session_id && s
        ? { label: "Continue in bg", hint: "The session is not running. A background job resumes it with this prompt." }
        : hasDir
          ? { label: "Start in bg", hint: "Starts the task as a background job with this prompt. Empty uses the title and the body." }
          : { label: "Start in bg", hint: "This card needs a project directory before it can run." };
  const canSend = !offline && !jobBusy && (running || hasDir) && (draft.trim() !== "" || (!running && !!(c.kind === "task" || c.run_prompt)));

  const send = () => {
    const text = draft.trim();
    if (!canSend) return;
    // An empty prompt on a task or a saved continue prompt starts the run as
    // the scheduler would: the title and the body, or the standing prompt.
    const fn = text ? () => api.sendPrompt(node, c.id, text) : () => api.startCard(node, c.id);
    // The composer empties only after the send goes through, so a failed send
    // costs a retry and not the prompt. The text the user typed while the send
    // was in flight is not the text that went, so that text stays.
    void onAction(fn, text ? "Sent" : "Started in background", undefined, picked).then((sent) => {
      setDraft((d) => (clearsBox(d, text, sent) ? "" : d));
      if (convOpen) loadConv();
    });
  };

  /** Send picked files to the node that owns the card. Each one is a call of
   *  its own, so a file that is refused does not take the rest with it. */
  const attach = async (list: FileList | File[] | null) => {
    const { files, refused } = await collectFiles(list);
    // A file the page will not send is reported here, before any call. The
    // drawer says everything through `onAction`, so a rejected promise is how
    // it raises an error toast.
    if (refused.length > 0) {
      const why = `${refused.join(", ")}. The limit is 4 MB per file.`;
      void onAction(() => Promise.reject(new Error(why)), undefined, undefined, picked);
    }
    if (files.length === 0) return;
    setAttachBusy(true);
    try {
      for (const f of files) {
        await onAction(
          async () => {
            const a = await api.addAttachment(node, c.id, f.name, f.dataUrl);
            setAdded((prev) => ({
              card: c.id,
              items: [...(prev?.card === c.id ? prev.items : attachments), a],
            }));
            return `Attached ${a.name}`;
          },
          "Attached",
          undefined,
          picked,
        );
      }
    } finally {
      setAttachBusy(false);
    }
  };

  const detach = (name: string) =>
    onAction(
      async () => {
        await api.deleteAttachment(node, c.id, name);
        setAdded({ card: c.id, items: attachments.filter((a) => a.name !== name) });
        return `Removed ${name}`;
      },
      "Removed",
      undefined,
      picked,
    );

  const putInComposer = (text: string) => {
    setDraft(text);
    draftGrow.ref.current?.focus();
  };

  return (
    <aside className="drawer" onInput={guard.asking ? guard.keep : undefined}>
      <button className="btn ghost sm close" onClick={close} aria-label="Close">
        ✕
      </button>
      <UnsavedBar guard={guard} text="The prompt you typed is not sent." />
      <header>
        <TitleEdit title={view.title} saved={c.title ?? ""} disabled={offline} onSave={(t) => patch({ title: t })} />
        <div className="hint">
          {showNode ? `${view.node_name} · ` : ""}
          {STATE_LABEL[view.state]} · {view.reason}
          {view.locked ? " · manual placement" : ""}
        </div>
        {offline && <div className="hint offline-note">This node is offline. Actions return when it reconnects.</div>}
        <div className="actions">
          {!mobile && (
            <button className="btn primary sm" disabled={offline} onClick={() => onAction(() => api.jumpIn(node, c.id), "Opened", undefined, picked)}>
              Jump in
            </button>
          )}
          {canBook && (
            <button
              className={`btn sm ${booked ? "primary" : ""}`}
              disabled={offline}
              title="Start this card at a set time, such as after the rate limit resets"
              onClick={() => setBooking(bookingOpen ? null : { card: c.id, at: "" })}
              aria-expanded={bookingOpen || !!booked}
            >
              ⏱ {booked ? `Booked ${clock(booked.at)}` : "Schedule"}
            </button>
          )}
          {bg?.state === "working" && (
            <button className="btn danger sm" disabled={offline} onClick={() => onAction(() => api.stopCard(node, c.id), "Stopped", undefined, picked)}>
              ■ Stop job
            </button>
          )}
          {doneCol && view.state !== "done" && (
            <button
              className="btn sm"
              disabled={offline}
              onClick={() =>
                onAction(
                  () => api.moveCard(node, c.id, doneCol.id),
                  "Marked done",
                  {
                    done: "Card moved back",
                    run: () => api.moveCard(node, c.id, view.column_id),
                  },
                  picked,
                )
              }
            >
              ✓ Done
            </button>
          )}
          {c.session_id && s && s.turns > 0 && (
            <button
              className="btn ghost sm"
              disabled={offline}
              title="Ask Haiku for a fresh summary now"
              onClick={() => onAction(() => api.summarizeCard(node, c.id), "Summary updated", undefined, picked)}
            >
              ✦ Summarize
            </button>
          )}
          <button
            className="btn ghost sm"
            disabled={offline}
            onClick={() =>
              onAction(
                () => api.patchCard(node, c.id, { archived: true }),
                "Archived",
                {
                  done: "Card back on the board",
                  run: () => api.patchCard(node, c.id, { archived: false }),
                },
                picked,
              ).then(onClose)
            }
          >
            Archive
          </button>
          {c.kind === "task" && (
            <button
              className="btn ghost sm"
              disabled={offline}
              onClick={() =>
                onAction(
                  () => api.deleteCard(node, c.id),
                  "Deleted",
                  {
                    done: "Card put back",
                    run: () => api.restoreCard(node, c),
                  },
                  picked,
                ).then(onClose)
              }
            >
              Delete
            </button>
          )}
        </div>
        {canBook && (bookingOpen || booked) && (
          <div className="sched">
            {booked && (
              <div className="sched-now">
                <b>⏱ {clock(booked.at)}</b>
                <span className="hint">
                  {booked.reason}
                  {untilTime(booked.at) ? ` · in ${untilTime(booked.at)}` : " · due"}
                  {booked.prompt ? " · with a one-off prompt" : ""}
                </span>
                <button
                  className="btn ghost sm"
                  disabled={offline}
                  onClick={() => onAction(() => api.cancelSchedule(node, c.id), "Booking cancelled", undefined, picked)}
                >
                  Cancel
                </button>
              </div>
            )}
            {bookingOpen && (
              <>
                <div className="hint">
                  kari starts this card at the time you pick, whatever the automation mode is. The times come from the
                  rate-limit sample of {view.node_name}. Text in the box at the foot goes with the booking as a one-off
                  prompt.
                </div>
                <div className="sched-opts">
                  {cycles.map((o) => (
                    <button
                      key={o.when}
                      className="btn sm"
                      disabled={offline || !preview[o.when]}
                      title={preview[o.when] ? clock(preview[o.when]) : "This node has no reset time for that window"}
                      onClick={() => book(o.when)}
                    >
                      {o.label}
                      <span className="at">{preview[o.when] ? clock(preview[o.when]) : "unknown"}</span>
                    </button>
                  ))}
                </div>
                <div className="sched-at">
                  <input
                    type="datetime-local"
                    value={bookAt}
                    onChange={(e) => setBooking({ card: c.id, at: e.target.value })}
                    aria-label="Start at this time"
                  />
                  <button
                    className="btn sm"
                    disabled={offline || !bookAt}
                    onClick={() => book("at", new Date(bookAt).toISOString())}
                  >
                    Book this time
                  </button>
                </div>
              </>
            )}
          </div>
        )}
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
                <button className="btn primary sm" onClick={() => onAction(() => api.answerPermission(node, view.permission!.id, "allow"), "Allowed", undefined, picked)}>
                  Allow
                </button>
                <button className="btn danger sm" onClick={() => onAction(() => api.answerPermission(node, view.permission!.id, "deny"), "Denied", undefined, picked)}>
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
                      <li key={o}>
                        <button className="linkbtn" disabled={offline} onClick={() => putInComposer(o)} title="Put this answer in the prompt box">
                          {o}
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            ))}
          </div>
        )}

        {(view.summary || bg) && (
          <div className="section">
            <h5>
              Where it stands
              {view.summary && (
                <span className="soft">
                  {" "}
                  · {view.summary.source} · {relTime(view.summary.generated_at)} ago · judged {STATE_LABEL[view.summary.judged_state]} ({Math.round(view.summary.confidence * 100)}%)
                </span>
              )}
            </h5>
            {view.summary?.delivery && <div className="delivery">{view.summary.delivery}</div>}
            {view.summary && <div className="narrative-block">{view.summary.narrative}</div>}
            {view.summary?.next_step && (
              <div className="hint" style={{ marginTop: 6 }}>
                Next: {view.summary.next_step}
              </div>
            )}
            {view.summary && view.summary.open_questions.length > 0 && q.length === 0 && (
              <ul className="soft-list">
                {view.summary.open_questions.map((x) => (
                  <li key={x}>{x}</li>
                ))}
              </ul>
            )}
            {bg && (
              <div className="jobstate">
                <span className={`st ${bg.state ?? ""}`}>job {bg.state ?? "?"}</span>
                {bg.detail && <span className="det">{bg.detail}</span>}
                {bg.needs && <div className="needs">Waits for: {bg.needs}</div>}
                {bg.waiting_for && !bg.needs && <div className="needs">Waits for: {bg.waiting_for}</div>}
                {bg.suggested_reply && (
                  <div className="suggest">
                    <span className="hint">The job suggests:</span>
                    <button className="btn sm" disabled={offline} onClick={() => putInComposer(bg.suggested_reply!)} title="Put this reply in the prompt box">
                      {bg.suggested_reply}
                    </button>
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        <div className="section">
          <h5>Card</h5>
          <dl className="kv edit">
            <dt>Project</dt>
            <dd>
              <ProjectPicker
                items={projectItems}
                value={cwd}
                allLabel={c.session_id ? "From the session" : "No project"}
                ariaLabel="Project directory"
                onChange={pickProject}
              />
              {cwd === OTHER && (
                <input
                  {...noAutoFill}
                  value={customCwd}
                  onChange={(e) => setCustomCwd(e.target.value)}
                  onBlur={saveCustomCwd}
                  onKeyDown={(e) => e.key === "Enter" && saveCustomCwd()}
                  placeholder="/absolute/path"
                />
              )}
              <div className="hint">
                {c.project_cwd
                  ? c.project_cwd
                  : c.session_id
                    ? `From the session${s?.cwd ? `: ${s.cwd}` : ""}.`
                    : "A run and Jump in both start here. The node refuses a path that is not a directory there."}
              </div>
            </dd>
            {nodes.length > 1 && (
              <>
                <dt>Node</dt>
                <dd>
                  {canMoveNode ? (
                    <>
                      <select value={moveTo} onChange={(e) => setPickedNode({ card: c.id, node: e.target.value })}>
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
                            Each node keeps its own cards, so a move writes the card again on {moveTarget?.name ?? moveTo} and gives it a new id. It keeps
                            the project only when that node holds one project of the same name.
                            {attachments.length > 0 &&
                              ` The ${attachments.length === 1 ? "attached file goes" : `${attachments.length} attached files go`} with it, copied to that node. If a file cannot be read, the card stays here.`}
                          </div>
                          <div style={{ marginTop: 6 }}>
                            <button className="btn sm" disabled={offline || moveTarget?.online === false} onClick={move}>
                              Move to {moveTarget?.name ?? moveTo}
                            </button>
                          </div>
                        </>
                      )}
                    </>
                  ) : (
                    <span title="This card follows a session on that node, so it cannot move.">{view.node_name}</span>
                  )}
                </dd>
              </>
            )}
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
            {bg && (
              <>
                <dt>Background</dt>
                <dd>
                  job {bg.id} · {bg.state}
                  {bg.waiting_for ? ` · waits: ${bg.waiting_for}` : ""}
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
            <dt>Runs</dt>
            <dd>
              <div className="runopts">
                <label>
                  <span>Priority</span>
                  <input
                    {...noAutoFill}
                    type="number"
                    value={c.priority}
                    disabled={offline}
                    onChange={(e) => patch({ priority: Number(e.target.value) })}
                    title="0 means automatic order. Dragging the card on the board writes this number."
                  />
                </label>
                <label>
                  <span>Model</span>
                  <select value={c.model ?? ""} disabled={offline} onChange={(e) => patch({ model: e.target.value })}>
                    {RUN_MODELS.map((m) => (
                      <option key={m.value} value={m.value}>
                        {m.label}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  <span>Permissions</span>
                  <select value={c.permission_mode ?? ""} disabled={offline} onChange={(e) => patch({ permission_mode: e.target.value })}>
                    {MODES.map((m) => (
                      <option key={m} value={m}>
                        {m || `default (${settings?.default_permission_mode ?? "auto"})`}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
              <label className="field inline" style={{ marginTop: 6 }}>
                <input type="checkbox" checked={c.auto_run} disabled={offline} onChange={(e) => patch({ auto_run: e.target.checked })} />
                <span>May run unattended when quota is left over</span>
              </label>
            </dd>
            <dt>{c.session_id ? "Scheduler" : "Body"}</dt>
            <dd>
              <textarea
                {...proseField}
                {...promptGrow}
                value={prompt.draft}
                disabled={offline}
                onChange={(e) => prompt.setDraft(e.target.value)}
                onBlur={prompt.flush}
                placeholder={c.session_id ? "Continue with the next step. Stop when done." : "Detail, links, where to start. The title is always the first line."}
              />
              <div className="hint">
                {c.session_id
                  ? "What an unattended run says when the scheduler resumes this session. To say something now, use the box at the foot."
                  : "A run receives the title, a blank line, then this body."}
              </div>
            </dd>
            <dt>Notes</dt>
            <dd>
              <textarea {...proseField} {...notesGrow} value={notes.draft} disabled={offline} onChange={(e) => notes.setDraft(e.target.value)} onBlur={notes.flush} placeholder="For you. A run never sees this." />
            </dd>
          </dl>
        </div>

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
                  <span className="det" title={l.detail ?? ""}>
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

        {c.session_id && s && (s.turns > 0 || s.last_assistant_text) && (
          <div className="section conversation">
            <h5>
              Conversation
              <span className="soft">
                {" "}
                · {conv ? `${conv.total} turns` : `${s.turns} prompts`}
              </span>
              <span className="spacer" />
              <button
                className="btn ghost sm"
                onClick={() => setConvOpen((o) => !o)}
                title={convOpen ? "Back to the last prompt and reply" : "Every prompt and reply, with a search"}
              >
                {convOpen ? "Latest only" : "Show all"}
              </button>
              {!mobile && (
                <button
                  className="btn ghost sm"
                  onClick={() => void popOutConversation(node, c.id, view.title)}
                  title="Open the conversation in its own window"
                  aria-label="Open the conversation in its own window"
                >
                  ⧉
                </button>
              )}
            </h5>
            {!convOpen && (
              <>
                {s.last_prompt && (
                  <div className="turn user">
                    <div className="who">
                      <span>you</span>
                      {s.last_user_at && <span className="when">{clock(s.last_user_at)}</span>}
                    </div>
                    <div className="quote">{s.last_prompt}</div>
                  </div>
                )}
                {s.last_assistant_text && (
                  <div className="turn assistant">
                    <div className="who">
                      <span>Claude</span>
                      {s.last_assistant_at && <span className="when">{clock(s.last_assistant_at)}</span>}
                    </div>
                    <Markdown className="quote md" text={s.last_assistant_text} />
                  </div>
                )}
              </>
            )}
            {convOpen && <ConversationList conv={conv} busy={convBusy} err={convErr} onLoadAll={() => loadConv(true)} />}
          </div>
        )}
      </div>
      <footer className="composer">
        <AttachmentList
          nodeId={node}
          cardId={c.id}
          items={attachments}
          disabled={offline || attachBusy}
          onRemove={detach}
        />
        <textarea
          {...proseField}
          {...draftGrow}
          value={draft}
          disabled={offline || jobBusy}
          onChange={(e) => setDraft(e.target.value)}
          onPaste={(e) => {
            // A screenshot on the clipboard arrives as a file here. Take it
            // and leave the text paste alone.
            const files = Array.from(e.clipboardData.files);
            if (files.length > 0) {
              e.preventDefault();
              void attach(files);
            }
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              send();
            }
          }}
          placeholder={running ? "Say something to the session… (⌘↵ sends)" : c.session_id && s ? "The next step for a background run… (⌘↵ sends)" : "A one-off prompt for the run… (⌘↵ sends)"}
          aria-label="Prompt"
        />
        <div className="composerrow">
          <span className="hint">
            {target.hint}
            {attachments.length > 0 &&
              (running
                ? ` The ${attachments.length === 1 ? "attached file is" : "attached files are"} named in the prompt. A running session asks for permission the first time it opens one.`
                : ` The run reads the ${attachments.length === 1 ? "attached file" : `${attachments.length} attached files`} from ${showNode ? view.node_name : "this node"}.`)}
          </span>
          <AttachButton
            disabled={offline || attachBusy}
            onPick={(l) => void attach(l)}
            label={attachBusy ? "Attaching…" : "Attach"}
          />
          <button className="btn primary sm" disabled={!canSend} onClick={send}>
            {draft.trim() || running ? target.label : `▶ ${target.label}`}
          </button>
        </div>
      </footer>
    </aside>
  );
}
