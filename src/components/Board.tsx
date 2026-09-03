import { DndContext, DragOverlay, PointerSensor, useDroppable, useSensor, useSensors, type DragEndEvent, type DragStartEvent } from "@dnd-kit/core";
import { useMemo, useState } from "react";
import type { CardView, Column } from "../types";
import { sortCards } from "../util";
import { CardItem } from "./CardItem";

interface Props {
  columns: Column[];
  cards: CardView[];
  selected: string | null;
  onSelect: (id: string | null) => void;
  onMove: (cardId: string, columnId: string) => void;
  onJump: (cardId: string) => void;
}

function ColumnView({ col, cards, selected, onSelect, onJump }: { col: Column; cards: CardView[]; selected: string | null; onSelect: (id: string) => void; onJump: (id: string) => void }) {
  const { setNodeRef, isOver } = useDroppable({ id: col.id });
  const over = col.wip_limit != null && cards.length > col.wip_limit;
  return (
    <div className={`col ${isOver ? "over" : ""}`} ref={setNodeRef}>
      <h4>
        <span className={`swatch ${col.color ?? "neutral"}`} />
        {col.name}
        <span className={`count ${over ? "over" : ""}`}>
          {cards.length}
          {col.wip_limit != null ? ` / ${col.wip_limit}` : ""}
        </span>
      </h4>
      <div className="cards">
        {cards.map((c) => (
          <CardItem key={c.card.id} view={c} selected={selected === c.card.id} onSelect={() => onSelect(c.card.id)} onJump={() => onJump(c.card.id)} />
        ))}
        {cards.length === 0 && <div className="empty">—</div>}
      </div>
    </div>
  );
}

export function Board({ columns, cards, selected, onSelect, onMove, onJump }: Props) {
  const [active, setActive] = useState<CardView | null>(null);
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }));

  const byColumn = useMemo(() => {
    const m = new Map<string, CardView[]>();
    for (const col of columns) m.set(col.id, []);
    for (const c of cards) {
      const arr = m.get(c.column_id);
      if (arr) arr.push(c);
    }
    for (const arr of m.values()) arr.sort(sortCards);
    return m;
  }, [columns, cards]);

  function onDragStart(e: DragStartEvent) {
    setActive(cards.find((c) => c.card.id === e.active.id) ?? null);
  }
  function onDragEnd(e: DragEndEvent) {
    const from = active;
    setActive(null);
    if (!from || !e.over) return;
    const to = String(e.over.id);
    if (to !== from.column_id) onMove(from.card.id, to);
  }

  return (
    <DndContext sensors={sensors} onDragStart={onDragStart} onDragEnd={onDragEnd} onDragCancel={() => setActive(null)}>
      <div className="board">
        {columns.map((col) => (
          <ColumnView key={col.id} col={col} cards={byColumn.get(col.id) ?? []} selected={selected} onSelect={onSelect} onJump={onJump} />
        ))}
      </div>
      <DragOverlay dropAnimation={null}>{active ? <CardItem view={active} selected={false} overlay /> : null}</DragOverlay>
    </DndContext>
  );
}
