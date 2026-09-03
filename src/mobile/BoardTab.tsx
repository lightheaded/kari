import { useMemo, useState } from "react";
import type { HubBoard } from "../types";
import { nodeDot, sortCards } from "../util";
import { MobileCard } from "./MobileCard";

interface Props {
  board: HubBoard;
  onOpen: (node: string, id: string) => void;
  onAction: (fn: () => Promise<unknown>, ok?: string) => Promise<void>;
}

/** One column at a time. The arrows or a swipe move between columns. */
export function BoardTab({ board, onOpen, onAction }: Props) {
  const columns = useMemo(() => board.columns.filter((c) => !c.hidden).sort((a, b) => a.order - b.order), [board.columns]);
  const [idx, setIdx] = useState(0);
  const [node, setNode] = useState("");
  const many = board.nodes.length > 1;
  const nodeById = useMemo(() => new Map(board.nodes.map((n) => [n.id, n])), [board.nodes]);
  const col = columns[Math.min(idx, columns.length - 1)];
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
    <div className="mboard">
      <header className="mhead">
        <button className="btn ghost sm" disabled={idx === 0} onClick={() => setIdx((i) => Math.max(0, i - 1))} aria-label="Previous column">
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
          onClick={() => setIdx((i) => Math.min(columns.length - 1, i + 1))}
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
