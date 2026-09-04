import type { NodeQueue } from "../types";
import { clock, fmtPct, untilTime } from "../util";

interface Props {
  queues: NodeQueue[];
  /** Show the node name on every step. Set when the board has more than one node. */
  showNode: boolean;
  open: boolean;
  onToggle: () => void;
  onSelectCard: (nodeId: string, cardId: string) => void;
}

/** When a step starts, in words the user can act on. */
function when(at: string | null, reason: string): string {
  if (!at) return reason;
  const inMs = new Date(at).getTime() - Date.now();
  if (inMs <= 60_000) return reason === "now" ? "now" : reason;
  if (inMs < 20 * 3600_000) return `in ${untilTime(at)}`;
  return clock(at);
}

/** A collapsible strip under the filter bar: what the planner would run next,
 *  in order, with what each step costs and when it would start. It starts
 *  nothing; the plan panel still holds the buttons. */
export function QueueStrip({ queues, showNode, open, onToggle, onSelectCard }: Props) {
  const live = queues.filter((q) => q.queue.steps.length > 0 || q.queue.blocked);
  if (live.length === 0) return null;

  const upNext = queues.reduce((n, q) => n + q.queue.steps.filter((s) => s.fits).length, 0);
  const soonest = queues
    .map((q) => q.queue.next_check_at)
    .filter(Boolean)
    .sort()[0];
  const blocked = queues.every((q) => q.queue.blocked);
  const why = blocked ? queues[0]?.queue.blocked : null;

  return (
    <div className={`queuestrip ${open ? "open" : ""}`}>
      <button className="qhead" onClick={onToggle} aria-expanded={open}>
        <span className="caret">{open ? "⌄" : "›"}</span>
        <b>Queue</b>
        <span className="qsum">
          {why ? why : `${upNext} up next`}
          {soonest && !why ? ` · next check ${untilTime(soonest) || "now"}` : ""}
        </span>
      </button>
      {open && (
        <div className="qbody">
          {live.map((q) => (
            <div className="qnode" key={q.node_id}>
              <div className="qnodehead">
                {showNode && <span className="who">{q.node_name}</span>}
                <span className="hint">
                  mode {q.queue.mode} · budget {fmtPct(q.queue.budget_pct)} · window at {fmtPct(q.queue.used_pct)}
                  {q.queue.next_trigger_at ? ` · trigger ${when(q.queue.next_trigger_at, "now")}` : ""}
                </span>
                {q.queue.open_proposal && <span className="qopen">a plan is open, waiting for Start</span>}
              </div>
              {q.queue.blocked && <div className="qblocked">{q.queue.blocked}</div>}
              <ol className="qsteps">
                {q.queue.steps.map((s, i) => (
                  <li key={s.card_id} className={s.fits ? "" : "no"}>
                    <span className="n">{i + 1}</span>
                    <button className="linkish" onClick={() => onSelectCard(q.node_id, s.card_id)} title="Open this card">
                      {s.title}
                    </button>
                    <span className="cost" title={`window at ${fmtPct(s.window_after_pct)} after this step`}>
                      {fmtPct(s.estimate.pct_five_hour)}
                    </span>
                    <span className="at">{when(s.starts_at, s.reason)}</span>
                  </li>
                ))}
              </ol>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
