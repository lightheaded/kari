import { useEffect, useMemo, useRef, useState } from "react";
import type { Act } from "../toasts";
import { api } from "../api";
import type { Column, HubCard } from "../types";
import { STATE_LABEL } from "../types";
import { STATE_TONE, clock, fmtM, relTime, weighted } from "../util";
import { NodeTag } from "../components/NodeTag";
import { PendingTurns } from "../components/Conversation";
import { NO_TURNS, isSending, trackSend, usePendingSends } from "../pending";

interface Props {
  view: HubCard;
  columns: Column[];
  showNode: boolean;
  /** The account that pays for the node. Set when the board spends more than one. */
  account?: string | null;
  offline: boolean;
  /** Show the answer buttons and the done button on the card itself. */
  actions: boolean;
  onOpen: () => void;
  onAction: Act;
}

/** One line about a tool call: the command, the file, the URL, or the first words. */
export function describeInput(tool: string, input: unknown): string {
  if (!input || typeof input !== "object") return typeof input === "string" ? input : "";
  const o = input as Record<string, unknown>;
  const keys = tool === "Bash" ? ["command"] : ["file_path", "notebook_path", "command", "url", "query", "pattern", "description"];
  for (const k of keys) {
    const v = o[k];
    if (typeof v === "string" && v.trim()) return v.replace(/\s+/g, " ").slice(0, 160);
  }
  return "";
}

/** A card the thumb can act on: tap an option, reply, mark done, stop, or open. */
export function MobileCard({ view, columns, showNode, account, offline, actions, onOpen, onAction }: Props) {
  const c = view.card;
  const node = view.node_id;
  const s = view.session;
  const tone = STATE_TONE[view.state];
  const q = s?.pending_tools.find((t) => t.name === "AskUserQuestion")?.questions[0];
  const bg = view.bg_job;
  const running = !!view.live?.alive;
  const [reply, setReply] = useState<string | null>(null);
  const doneCol = columns.find((k) => k.accepts.includes("done"));
  // A running session takes a reply into its own queue. Anything else needs a
  // directory to start a background job in. The box opens either way, as it
  // does in the card sheet, so an answer can be typed and kept. The send waits
  // while a job holds the card and its session does not answer yet, because a
  // second run in the same directory would write into the same transcript.
  const canReply = running || !!(c.project_cwd ?? s?.cwd);
  const jobBusy = !running && bg?.state === "working";
  const canSend = canReply && !jobBusy;
  const perm = view.permission ?? null;
  const permText = perm ? describeInput(perm.tool_name, perm.tool_input) : "";

  /** The card knows only the last prompt of its session, and that is enough
   *  to see that the session read a prompt that was sent. */
  const lastPrompt = s?.last_prompt;
  const lastPromptAt = s?.last_user_at ?? null;
  const seen = useMemo(() => (lastPrompt ? [{ role: "user", text: lastPrompt, at: lastPromptAt }] : NO_TURNS), [lastPrompt, lastPromptAt]);
  const pending = usePendingSends(node, c.id, seen);
  const sending = isSending(pending);
  /** The box as it is now, for a send that fails after the user typed again. */
  const replyNow = useRef(reply);
  useEffect(() => {
    replyNow.current = reply;
  }, [reply]);

  // The box closes at once, and the message shows under the card until the
  // session reads it. A phone sends over a link that drops, so a failed send
  // opens the box again with the message in it: it costs a retry and not the
  // message. The error itself arrives as a toast, from `onAction`.
  const send = async (text: string) => {
    if (sending) return;
    setReply(null);
    const restore = (t: string) => {
      if ((replyNow.current ?? "").trim() !== "") return false;
      setReply(t);
      return true;
    };
    await onAction(() => trackSend(node, c.id, text, () => api.sendPrompt(node, c.id, text), restore), "Sent", undefined, { node, id: c.id });
  };

  return (
    <div className={`mcard tone-${tone} ${offline ? "offline" : ""}`}>
      <button className="mcard-head" onClick={onOpen}>
        {showNode && (
          <div className="mcard-node">
            <NodeTag nodeId={view.node_id} nodeName={view.node_name} account={account} online={!offline} />
          </div>
        )}
        <div className="mtitle">{view.title}</div>
        <div className="mmeta">
          <b>{STATE_LABEL[view.state]}</b>
          {view.project_name ? ` · ${view.project_name}` : ""}
          {view.last_activity_at ? ` · ${relTime(view.last_activity_at)}` : ""}
          {s && weighted(s.tokens) > 0 ? ` · ${fmtM(weighted(s.tokens))}` : ""}
          {bg?.state ? ` · bg ${bg.state}` : ""}
          {c.scheduled ? ` · ⏱ ${clock(c.scheduled.at)}` : ""}
          {view.attachments?.length ? ` · 📎 ${view.attachments.length}` : ""}
          {offline ? " · node offline" : ""}
          {view.pending ? " · waiting for node" : ""}
        </div>
        {view.hooks?.blocked_command && (
          <div className="mblocked">Stopped short of a {view.hooks.blocked_kind ?? "release"}: {view.hooks.blocked_command}</div>
        )}
        {view.summary?.narrative && !q && actions && <div className="mnarr">{view.summary.narrative}</div>}
        {bg?.waiting_for && <div className="mq">{bg.waiting_for}</div>}
        {q && <div className="mq">{q.question}</div>}
        {perm && (
          <div className="mq">
            <b>{perm.tool_name}</b> asks for permission{permText ? `: ${permText}` : ""}
            <div className="mhint">held until {new Date(perm.until).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</div>
          </div>
        )}
      </button>

      {perm && !offline && (
        <div className="macts">
          <button className="btn primary sm" onClick={() => onAction(() => api.answerPermission(node, perm.id, "allow"), "Allowed")}>
            Allow
          </button>
          <button className="btn danger sm" onClick={() => onAction(() => api.answerPermission(node, perm.id, "deny"), "Denied")}>
            Deny
          </button>
        </div>
      )}

      {actions && !offline && (
        <div className="macts">
          {q?.options.slice(0, 4).map((o) => (
            <button key={o} className="btn sm" disabled={!canSend || sending} onClick={() => void send(o)}>
              {o}
            </button>
          ))}
          {canReply && (
            <button className="btn sm" onClick={() => setReply(reply === null ? "" : null)}>
              Reply…
            </button>
          )}
          {bg?.state === "working" && (
            <button className="btn danger sm" onClick={() => onAction(() => api.stopCard(node, c.id), "Stopped")}>
              ■ Stop
            </button>
          )}
          {doneCol && view.state !== "done" && (
            <button className="btn sm" onClick={() => onAction(() => api.moveCard(node, c.id, doneCol.id), "Marked done")}>
              ✓ Done
            </button>
          )}
          <button className="btn ghost sm" onClick={onOpen}>
            Open
          </button>
        </div>
      )}

      {reply !== null && (
        <div className="mreply">
          <textarea value={reply} onChange={(e) => setReply(e.target.value)} placeholder="Tell the agent what to do next" rows={3} />
          {jobBusy && <div className="mhint">A background job holds this card. The text stays here until the job stops or its session answers.</div>}
          <div className="macts">
            <button className="btn primary sm" disabled={!reply.trim() || !canSend || sending} onClick={() => void send(reply.trim())}>
              {sending ? "Sending…" : "Send"}
            </button>
            <button className="btn ghost sm" disabled={sending} onClick={() => setReply(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}

      <PendingTurns list={pending} />

      {view.state === "needs_approval" && actions && !bg && !perm && (
        <div className="mhint">A permission prompt waits in the terminal. Turn on Away mode for {view.node_name} in Nodes to answer the next one here.</div>
      )}
    </div>
  );
}
