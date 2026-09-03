import { closestCorners, DndContext, DragOverlay, PointerSensor, useDroppable, useSensor, useSensors, type DragEndEvent, type DragStartEvent } from "@dnd-kit/core";
import { arrayMove, SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { useEffect, useMemo, useRef, useState } from "react";
import type { CardView, Column } from "../types";
import { STATE_HELP, STATE_LABEL } from "../types";
import { sortCards } from "../util";
import { CardItem } from "./CardItem";

interface Props {
  columns: Column[];
  cards: CardView[];
  selected: string | null;
  onSelect: (id: string | null) => void;
  onMove: (cardId: string, columnId: string) => void;
  /** The cards of one column in their new order, top first. */
  onReorder: (cardIds: string[]) => void;
  onJump: (cardId: string) => void;
}

function columnHelp(col: Column): string {
  if (col.accepts.length === 0) return "This column accepts no derived state. Cards land here only by a manual move.";
  const lines = col.accepts.map((s) => `${STATE_LABEL[s]}: ${STATE_HELP[s]}`);
  return `Accepts ${col.accepts.length === 1 ? "one state" : `${col.accepts.length} states`}.\n\n${lines.join("\n\n")}`;
}

function ColumnView({ col, cards, selected, onSelect, onJump }: { col: Column; cards: CardView[]; selected: string | null; onSelect: (id: string) => void; onJump: (id: string) => void }) {
  const { setNodeRef, isOver } = useDroppable({ id: col.id, data: { type: "column", columnId: col.id } });
  const over = col.wip_limit != null && cards.length > col.wip_limit;
  return (
    <div className={`col ${isOver ? "over" : ""}`} ref={setNodeRef}>
      <h4 title={columnHelp(col)}>
        <span className={`swatch ${col.color ?? "neutral"}`} />
        {col.name}
        <span className={`count ${over ? "over" : ""}`} title={col.wip_limit != null ? `${cards.length} of a limit of ${col.wip_limit}` : `${cards.length} cards`}>
          {cards.length}
          {col.wip_limit != null ? ` / ${col.wip_limit}` : ""}
        </span>
      </h4>
      <SortableContext items={cards.map((c) => c.card.id)} strategy={verticalListSortingStrategy}>
        <div className="cards">
          {cards.map((c) => (
            <CardItem key={c.card.id} view={c} selected={selected === c.card.id} onSelect={() => onSelect(c.card.id)} onJump={() => onJump(c.card.id)} />
          ))}
          {cards.length === 0 && <div className="empty">—</div>}
        </div>
      </SortableContext>
    </div>
  );
}

export function Board({ columns, cards, selected, onSelect, onMove, onReorder, onJump }: Props) {
  const [active, setActive] = useState<CardView | null>(null);
  // The order the user just dropped, shown until the board comes back with new cards.
  const [dropped, setPending] = useState<{ columnId: string; ids: string[]; cards: CardView[] } | null>(null);
  const pending = dropped && dropped.cards === cards ? dropped : null;
  const boardRef = useRef<HTMLDivElement>(null);
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }));

  const byColumn = useMemo(() => {
    const m = new Map<string, CardView[]>();
    for (const col of columns) m.set(col.id, []);
    for (const c of cards) {
      const arr = m.get(c.column_id);
      if (arr) arr.push(c);
    }
    for (const arr of m.values()) arr.sort(sortCards);
    if (pending) {
      const arr = m.get(pending.columnId);
      if (arr && arr.length === pending.ids.length && arr.every((c) => pending.ids.includes(c.card.id))) {
        arr.sort((a, b) => pending.ids.indexOf(a.card.id) - pending.ids.indexOf(b.card.id));
      }
    }
    return m;
  }, [columns, cards, pending]);

  // A mouse wheel only scrolls up and down. Over a column that cannot scroll
  // further, that motion moves the board sideways instead. Trackpads and
  // Shift+wheel already send a horizontal delta, and those pass through.
  useEffect(() => {
    const el = boardRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (e.shiftKey || e.ctrlKey || e.metaKey) return;
      if (Math.abs(e.deltaX) >= Math.abs(e.deltaY) || e.deltaY === 0) return;
      const list = (e.target as HTMLElement | null)?.closest?.(".cards") as HTMLElement | null;
      if (list) {
        const canDown = list.scrollTop + list.clientHeight < list.scrollHeight - 1;
        const canUp = list.scrollTop > 0;
        if ((e.deltaY > 0 && canDown) || (e.deltaY < 0 && canUp)) return;
      }
      if (el.scrollWidth <= el.clientWidth) return;
      e.preventDefault();
      el.scrollLeft += e.deltaY;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  function onDragStart(e: DragStartEvent) {
    setActive(cards.find((c) => c.card.id === e.active.id) ?? null);
  }
  function onDragEnd(e: DragEndEvent) {
    const from = active;
    setActive(null);
    if (!from || !e.over) return;
    const overData = e.over.data.current as { type?: string; columnId?: string } | undefined;
    const to = overData?.columnId ?? String(e.over.id);
    if (to !== from.column_id) {
      onMove(from.card.id, to);
      return;
    }
    if (overData?.type !== "card" || e.over.id === e.active.id) return;
    const ids = (byColumn.get(to) ?? []).map((c) => c.card.id);
    const oldIndex = ids.indexOf(String(e.active.id));
    const newIndex = ids.indexOf(String(e.over.id));
    if (oldIndex < 0 || newIndex < 0 || oldIndex === newIndex) return;
    const next = arrayMove(ids, oldIndex, newIndex);
    setPending({ columnId: to, ids: next, cards });
    onReorder(next);
  }

  return (
    <DndContext sensors={sensors} collisionDetection={closestCorners} onDragStart={onDragStart} onDragEnd={onDragEnd} onDragCancel={() => setActive(null)}>
      <div className="board" ref={boardRef}>
        {columns.map((col) => (
          <ColumnView key={col.id} col={col} cards={byColumn.get(col.id) ?? []} selected={selected} onSelect={onSelect} onJump={onJump} />
        ))}
      </div>
      <DragOverlay dropAnimation={null}>{active ? <CardItem view={active} selected={false} overlay /> : null}</DragOverlay>
    </DndContext>
  );
}
