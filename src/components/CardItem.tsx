import { useSortable } from "@dnd-kit/sortable";
import type { HubCard } from "../types";
import { STATE_LABEL } from "../types";
import { STATE_TONE, fmtM, fmtPct, relTime, weighted } from "../util";

interface Props {
  view: HubCard;
  selected: boolean;
  overlay?: boolean;
  /** Show which node the card comes from. Set when the board has more than one node. */
  showNode?: boolean;
  /** The node does not answer. The card is dimmed and cannot be dragged. */
  offline?: boolean;
  lastSeen?: string | null;
  onSelect?: () => void;
  onJump?: () => void;
  /** A click on the node chip filters the board to that node. */
  onFilterNode?: () => void;
}

export function CardItem({ view, selected, overlay, showNode, offline, lastSeen, onSelect, onJump, onFilterNode }: Props) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: `${view.node_id}/${view.card.id}`,
    data: { type: "card", columnId: view.column_id },
    disabled: overlay || offline,
  });
  const tone = STATE_TONE[view.state];
  const s = view.session;
  const tokens = weighted(s?.tokens);
  const q = s?.pending_tools.find((t) => t.name === "AskUserQuestion")?.questions[0];
  const live = view.live;
  const bg = view.bg_job;
  const ranked = view.card.priority !== 0;

  return (
    <div
      ref={setNodeRef}
      style={overlay ? undefined : { transform: transform ? `translate3d(0, ${transform.y}px, 0)` : undefined, transition }}
      className={`card tone-${tone} ${selected ? "selected" : ""} ${isDragging || overlay ? "dragging" : ""} ${offline ? "offline" : ""}`}
      {...listeners}
      {...attributes}
      onClick={onSelect}
      onDoubleClick={(e) => {
        e.stopPropagation();
        onJump?.();
      }}
      title={offline ? `node offline, last seen ${lastSeen ? `${relTime(lastSeen)} ago` : "never"}` : "Click for details, double-click to open the session, drag to reorder"}
    >
      {view.locked && <span className="lock" title="Manual placement. Holds until a stronger signal.">⌖</span>}
      {ranked && !view.locked && (
        <span className="rank" title={`Placed by hand at priority ${view.card.priority}. Cards you never dragged sort below it.`}>
          ≡
        </span>
      )}
      <div className="title">{view.title}</div>
      {view.summary?.narrative && !q && <div className="narrative">{view.summary.narrative}</div>}
      <div className="chips">
        <span className="chip">
          {(live?.alive || bg?.state === "working") && <span className={`live ${view.state === "working" ? "pulse" : ""}`} />}
          {STATE_LABEL[view.state]}
        </span>
        {showNode && (
          <button
            className="chip node act"
            title={`Show only the cards of ${view.node_name}`}
            onClick={(e) => {
              e.stopPropagation();
              onFilterNode?.();
            }}
            // A drag must start on the card, not on this button.
            onPointerDown={(e) => e.stopPropagation()}
          >
            {view.node_name}
          </button>
        )}
        {view.card.kind === "task" && <span className="chip plain">task</span>}
        {view.card.kind === "session" && !live && !bg && view.state !== "done" && <span className="chip plain">exited</span>}
        {view.card.auto_run && <span className="chip plain">auto-run</span>}
        {view.card.model && <span className="chip plain" title="Model used when this card starts">{view.card.model}</span>}
        {view.card.kind === "task" && view.estimate && (
          <span className="chip plain" title={`estimate ${fmtM(view.estimate.weighted_tokens)} weighted tokens (${view.estimate.source})`}>
            ~{fmtPct(view.estimate.pct_five_hour)}
          </span>
        )}
        {bg && <span className="chip plain">bg {bg.state ?? ""}</span>}
        {view.herdr && <span className="chip plain">herdr {view.herdr.pane_id}</span>}
        {s?.pr_links.length ? <span className="chip plain">PR</span> : null}
      </div>
      {q && (
        <div className="question">
          {q.question}
          {q.options.length > 0 && (
            <div className="opts">
              {q.options.slice(0, 4).map((o) => (
                <span key={o}>{o}</span>
              ))}
            </div>
          )}
        </div>
      )}
      <div className="row">
        <span className="proj">{view.project_name ?? ""}</span>
        <span>
          {view.last_activity_at ? relTime(view.last_activity_at) : ""}
          {tokens > 0 ? ` · ${fmtM(tokens)}` : ""}
        </span>
      </div>
    </div>
  );
}
