import { DndContext, DragOverlay, PointerSensor, useDroppable, useSensor, useSensors, type DragEndEvent, type DragStartEvent } from "@dnd-kit/core";
import { useMemo, useState } from "react";
import type { Column, HubCard, NodeStatus } from "../types";
import { sortCards } from "../util";
import { CardItem } from "./CardItem";

export interface Picked {
  node: string;
  id: string;
}

interface Props {
  columns: Column[];
  cards: HubCard[];
  nodes: NodeStatus[];
  selected: Picked | null;
  onSelect: (nodeId: string, cardId: string) => void;
  onMove: (nodeId: string, cardId: string, columnId: string) => void;
  onJump: (nodeId: string, cardId: string) => void;
}

interface ColProps {
  col: Column;
  cards: HubCard[];
  nodes: Map<string, NodeStatus>;
  showNode: boolean;
  selected: Picked | null;
  onSelect: (nodeId: string, cardId: string) => void;
  onJump: (nodeId: string, cardId: string) => void;
}

function ColumnView({ col, cards, nodes, showNode, selected, onSelect, onJump }: ColProps) {
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
        {cards.map((c) => {
          const node = nodes.get(c.node_id);
          return (
            <CardItem
              key={`${c.node_id}/${c.card.id}`}
              view={c}
              selected={selected?.node === c.node_id && selected?.id === c.card.id}
              showNode={showNode}
              offline={node ? !node.online : false}
              lastSeen={node?.last_seen ?? null}
              onSelect={() => onSelect(c.node_id, c.card.id)}
              onJump={() => onJump(c.node_id, c.card.id)}
            />
          );
        })}
        {cards.length === 0 && <div className="empty">—</div>}
      </div>
    </div>
  );
}

export function Board({ columns, cards, nodes, selected, onSelect, onMove, onJump }: Props) {
  const [active, setActive] = useState<HubCard | null>(null);
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }));
  const byId = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);
  const showNode = nodes.length > 1;

  const byColumn = useMemo(() => {
    const m = new Map<string, HubCard[]>();
    for (const col of columns) m.set(col.id, []);
    for (const c of cards) {
      const arr = m.get(c.column_id);
      if (arr) arr.push(c);
    }
    for (const arr of m.values()) arr.sort(sortCards);
    return m;
  }, [columns, cards]);

  function onDragStart(e: DragStartEvent) {
    const key = String(e.active.id);
    setActive(cards.find((c) => `${c.node_id}/${c.card.id}` === key) ?? null);
  }
  function onDragEnd(e: DragEndEvent) {
    const from = active;
    setActive(null);
    if (!from || !e.over) return;
    const to = String(e.over.id);
    if (to !== from.column_id) onMove(from.node_id, from.card.id, to);
  }

  return (
    <DndContext sensors={sensors} onDragStart={onDragStart} onDragEnd={onDragEnd} onDragCancel={() => setActive(null)}>
      <div className="board">
        {columns.map((col) => (
          <ColumnView
            key={col.id}
            col={col}
            cards={byColumn.get(col.id) ?? []}
            nodes={byId}
            showNode={showNode}
            selected={selected}
            onSelect={onSelect}
            onJump={onJump}
          />
        ))}
      </div>
      <DragOverlay dropAnimation={null}>{active ? <CardItem view={active} selected={false} showNode={showNode} overlay /> : null}</DragOverlay>
    </DndContext>
  );
}
