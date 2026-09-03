import { useDraggable } from "@dnd-kit/core";
import type { CardView } from "../types";
import { STATE_LABEL } from "../types";
import { STATE_TONE, fmtM, fmtPct, relTime, weighted } from "../util";

interface Props {
  view: CardView;
  selected: boolean;
  overlay?: boolean;
  onSelect?: () => void;
  onJump?: () => void;
}

export function CardItem({ view, selected, overlay, onSelect, onJump }: Props) {
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({ id: view.card.id, disabled: overlay });
  const tone = STATE_TONE[view.state];
  const s = view.session;
  const tokens = weighted(s?.tokens);
  const q = s?.pending_tools.find((t) => t.name === "AskUserQuestion")?.questions[0];
  const live = view.live;
  const bg = view.bg_job;

  return (
    <div
      ref={setNodeRef}
      className={`card tone-${tone} ${selected ? "selected" : ""} ${isDragging || overlay ? "dragging" : ""}`}
      {...listeners}
      {...attributes}
      onClick={onSelect}
      onDoubleClick={(e) => {
        e.stopPropagation();
        onJump?.();
      }}
      title="Click for details, double-click to open the session"
    >
      {view.locked && <span className="lock" title="Manual placement. Holds until a stronger signal.">⌖</span>}
      <div className="title">{view.title}</div>
      {view.summary?.narrative && !q && <div className="narrative">{view.summary.narrative}</div>}
      <div className="chips">
        <span className="chip">
          {(live?.alive || bg?.state === "working") && <span className={`live ${view.state === "working" ? "pulse" : ""}`} />}
          {STATE_LABEL[view.state]}
        </span>
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
