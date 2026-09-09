import { useMemo, useRef, useState } from "react";
import type { Act } from "../toasts";
import type { HubBoard } from "../types";
import { nodeDot, sortCards } from "../util";
import { MobileCard } from "./MobileCard";

/** How far a finger must travel sideways before it counts as a swipe, and how
 *  much more sideways than up or down. A scroll of the list moves up and down;
 *  a swipe that also drifted a little must still turn the page. */
const SWIPE_PX = 48;
const SWIPE_RATIO = 1.4;

interface Props {
  board: HubBoard;
  onOpen: (node: string, id: string) => void;
  onAction: Act;
}

/** One column at a time. The arrows or a swipe move between columns. The
 *  first view is the column that holds the sessions at work, because that is
 *  what a phone is opened to check. */
export function BoardTab({ board, onOpen, onAction }: Props) {
  const columns = useMemo(() => board.columns.filter((c) => !c.hidden).sort((a, b) => a.order - b.order), [board.columns]);
  /** The column on screen, by id, so a reordered board keeps the page. Null
   *  until the first board arrives with a Working column to start on. */
  const [colId, setColId] = useState<string | null>(null);
  const [node, setNode] = useState("");
  const many = board.nodes.length > 1;
  const nodeById = useMemo(() => new Map(board.nodes.map((n) => [n.id, n])), [board.nodes]);
  const start = Math.max(
    0,
    columns.findIndex((c) => c.accepts.includes("working")),
  );
  const found = colId ? columns.findIndex((c) => c.id === colId) : -1;
  const idx = found >= 0 ? found : Math.min(start, Math.max(0, columns.length - 1));
  const setIdx = (i: number) => {
    const c = columns[Math.max(0, Math.min(columns.length - 1, i))];
    if (c) setColId(c.id);
  };
  const col = columns[idx];
  /** Where the finger came down, for the swipe. */
  const touch = useRef<{ x: number; y: number } | null>(null);
  const onTouchStart = (e: React.TouchEvent) => {
    const t = e.touches[0];
    touch.current = t ? { x: t.clientX, y: t.clientY } : null;
  };
  const onTouchEnd = (e: React.TouchEvent) => {
    const from = touch.current;
    touch.current = null;
    const t = e.changedTouches[0];
    if (!from || !t) return;
    const dx = t.clientX - from.x;
    const dy = t.clientY - from.y;
    if (Math.abs(dx) < SWIPE_PX || Math.abs(dx) < Math.abs(dy) * SWIPE_RATIO) return;
    setIdx(idx + (dx < 0 ? 1 : -1));
  };
  const cards = useMemo(
    () => board.cards.filter((c) => c.column_id === col?.id && (!node || c.node_id === node) && !c.card.archived).sort(sortCards),
    [board.cards, col, node],
  );
  const counts = useMemo(() => {
    const m = new Map<string, number>();
    for (const c of board.cards) {
      if (c.card.archived || (node && c.node_id !== node)) continue;
      m.set(c.column_id, (m.get(c.column_id) ?? 0) + 1);
    }
    return m;
  }, [board.cards, node]);

  if (!col) return <div className="empty">No columns yet.</div>;

  return (
    <div className="mboard" onTouchStart={onTouchStart} onTouchEnd={onTouchEnd}>
      <header className="mhead">
        <button className="btn ghost sm" disabled={idx === 0} onClick={() => setIdx(idx - 1)} aria-label="Previous column">
          ‹
        </button>
        <span className={`mcol tone-${col.color ?? "neutral"}`}>
          <span className="swatch" />
          {col.name}
          <span className="mcount">{counts.get(col.id) ?? 0}</span>
        </span>
        <button
          className="btn ghost sm"
          disabled={idx >= columns.length - 1}
          onClick={() => setIdx(idx + 1)}
          aria-label="Next column"
        >
          ›
        </button>
      </header>
      <div className="mcolstrip">
        {columns.map((c, i) => (
          <button key={c.id} className={`mdot ${i === idx ? "on" : ""}`} onClick={() => setIdx(i)} aria-label={c.name} />
        ))}
      </div>
      {many && (
        <div className="nodechips mchips">
          <button className={`nodechip ${node === "" ? "sel" : ""}`} onClick={() => setNode("")}>
            All nodes
          </button>
          {board.nodes.map((n) => (
            <button key={n.id} className={`nodechip ${node === n.id ? "sel" : ""}`} onClick={() => setNode(n.id)}>
              <span className={nodeDot(n)} />
              {n.name}
            </button>
          ))}
        </div>
      )}
      {cards.length === 0 ? (
        <div className="empty">No cards in {col.name}.</div>
      ) : (
        <div className="mlist">
          {cards.map((c) => (
            <MobileCard
              key={`${c.node_id}/${c.card.id}`}
              view={c}
              columns={board.columns}
              showNode={many}
              offline={nodeById.get(c.node_id)?.online === false}
              actions={false}
              onOpen={() => onOpen(c.node_id, c.card.id)}
              onAction={onAction}
            />
          ))}
        </div>
      )}
    </div>
  );
}
