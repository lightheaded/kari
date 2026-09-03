import { useState } from "react";
import { api } from "../api";
import type { Column, HubCard } from "../types";
import { STATE_LABEL } from "../types";
import { STATE_TONE, fmtM, relTime, weighted } from "../util";

interface Props {
  view: HubCard;
  columns: Column[];
  showNode: boolean;
  offline: boolean;
  /** Show the answer buttons and the done button on the card itself. */
  actions: boolean;
  onOpen: () => void;
  onAction: (fn: () => Promise<unknown>, ok?: string) => Promise<void>;
}

/** A card the thumb can act on: tap an option, reply, mark done, stop, or open. */
export function MobileCard({ view, columns, showNode, offline, actions, onOpen, onAction }: Props) {
  const c = view.card;
  const node = view.node_id;
  const s = view.session;
  const tone = STATE_TONE[view.state];
  const q = s?.pending_tools.find((t) => t.name === "AskUserQuestion")?.questions[0];
  const bg = view.bg_job;
  const liveInTerminal = !!view.live?.alive && !bg;
  const [reply, setReply] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const doneCol = columns.find((k) => k.accepts.includes("done"));
  const canReply = !!(c.project_cwd ?? s?.cwd) && bg?.state !== "working";

  /** A live terminal session gets a warning first: a second process writes into the same transcript. */
  const send = (text: string) => {
    if (liveInTerminal && pending !== text) {
      setPending(text);
      return;
    }
    setPending(null);
    setReply(null);
    onAction(() => api.startCard(node, c.id, text), "Sent to the agent");
  };

  return (
    <div className={`mcard tone-${tone} ${offline ? "offline" : ""}`}>
      <button className="mcard-head" onClick={onOpen}>
        <div className="mtitle">{view.title}</div>
        <div className="mmeta">
          {showNode ? `${view.node_name} · ` : ""}
          <b>{STATE_LABEL[view.state]}</b>
          {view.project_name ? ` · ${view.project_name}` : ""}
          {view.last_activity_at ? ` · ${relTime(view.last_activity_at)}` : ""}
          {s && weighted(s.tokens) > 0 ? ` · ${fmtM(weighted(s.tokens))}` : ""}
          {bg?.state ? ` · bg ${bg.state}` : ""}
          {offline ? " · node offline" : ""}
        </div>
        {view.summary?.narrative && !q && actions && <div className="mnarr">{view.summary.narrative}</div>}
        {bg?.waiting_for && <div className="mq">{bg.waiting_for}</div>}
        {q && <div className="mq">{q.question}</div>}
      </button>

      {actions && !offline && (
        <div className="macts">
          {q?.options.slice(0, 4).map((o) => (
            <button key={o} className="btn sm" disabled={!canReply} onClick={() => send(o)}>
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
          <div className="macts">
            <button className="btn primary sm" disabled={!reply.trim()} onClick={() => send(reply.trim())}>
              Send
            </button>
            <button className="btn ghost sm" onClick={() => setReply(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {pending !== null && (
        <div className="mwarn">
          This session is open in a terminal on {view.node_name}. A reply from here continues it as a background job, and both write into one transcript.
          <div className="macts">
            <button className="btn danger sm" onClick={() => send(pending)}>
              Reply anyway
            </button>
            <button className="btn ghost sm" onClick={() => setPending(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {view.state === "needs_approval" && actions && !bg && (
        <div className="mhint">A permission prompt waits in the terminal. Answering it from here comes with Away mode.</div>
      )}
    </div>
  );
}
