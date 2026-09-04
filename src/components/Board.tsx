import {
  closestCorners,
  DndContext,
  DragOverlay,
  PointerSensor,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Column, DerivedState, HubCard, NodeStatus } from "../types";
import { STATE_HELP, STATE_LABEL } from "../types";
import { clearRanks, planReorder, sortCards, STATE_TONE } from "../util";
import { CardItem } from "./CardItem";
import { ColumnAdd } from "./ColumnAdd";

export interface Picked {
  node: string;
  id: string;
}

/** The cards of one column in their new order, and how the two parts split. */
export interface Reorder {
  node: string;
  columnId: string;
  ranked: string[];
  unranked: string[];
}

interface Props {
  columns: Column[];
  cards: HubCard[];
  nodes: NodeStatus[];
  selected: Picked | null;
  onSelect: (nodeId: string, cardId: string) => void;
  onMove: (nodeId: string, cardId: string, columnId: string) => void;
  onReorder: (r: Reorder) => void;
  onJump: (nodeId: string, cardId: string) => void;
  onFilterNode: (nodeId: string) => void;
  onAdd: (columnId: string, title: string) => Promise<void>;
  onAddFull: (columnId: string) => void;
}

/** The keys `planReorder` and `clearRanks` read, for one card. */
const rankable = (c: HubCard) => ({
  key: `${c.node_id}/${c.card.id}`,
  node: c.node_id,
  id: c.card.id,
  priority: c.card.priority,
});

function columnHelp(col: Column): string {
  if (col.accepts.length === 0) return "No derived state lands here. Cards arrive by a manual move only.";
  return col.accepts.map((s) => `${STATE_LABEL[s]}: ${STATE_HELP[s]}`).join("\n\n");
}

/** Split the cards of a merged column into one group per state it accepts, in
 *  the order the column lists them. A column with one state, or with cards of
 *  one state only, stays a flat list. */
function groupCards(col: Column, cards: HubCard[]): { state: DerivedState | null; cards: HubCard[] }[] {
  if (col.accepts.length < 2) return [{ state: null, cards }];
  const seen = new Set(cards.map((c) => c.state));
  if (seen.size < 2) return [{ state: null, cards }];
  const out: { state: DerivedState | null; cards: HubCard[] }[] = [];
  for (const s of col.accepts) {
    const part = cards.filter((c) => c.state === s);
    if (part.length > 0) out.push({ state: s, cards: part });
  }
  // A card whose state the column no longer accepts still has to appear.
  const rest = cards.filter((c) => !col.accepts.includes(c.state));
  if (rest.length > 0) out.push({ state: null, cards: rest });
  return out;
}

interface ColProps {
  col: Column;
  cards: HubCard[];
  nodes: Map<string, NodeStatus>;
  showNode: boolean;
  selected: Picked | null;
  collapsed: Set<string>;
  onCollapse: (key: string) => void;
  onSelect: (nodeId: string, cardId: string) => void;
  onJump: (nodeId: string, cardId: string) => void;
  onFilterNode: (nodeId: string) => void;
  onAdd: (title: string) => Promise<void>;
  onAddFull: () => void;
  /** Give every placed card of this column back to the automatic order. */
  onClearRanks: () => void;
}

function ColumnView({
  col,
  cards,
  nodes,
  showNode,
  selected,
  collapsed,
  onCollapse,
  onSelect,
  onJump,
  onFilterNode,
  onAdd,
  onAddFull,
  onClearRanks,
}: ColProps) {
  const { setNodeRef, isOver } = useDroppable({ id: col.id, data: { type: "column", columnId: col.id } });
  const over = col.wip_limit != null && cards.length > col.wip_limit;
  const groups = groupCards(col, cards);
  const placed = cards.some((c) => c.card.priority !== 0);

  const one = (c: HubCard) => {
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
        onFilterNode={() => onFilterNode(c.node_id)}
      />
    );
  };

  return (
    <div className={`col ${isOver ? "over" : ""}`} ref={setNodeRef}>
      <h4 title={columnHelp(col)}>
        <span className={`swatch ${col.color ?? "neutral"}`} />
        {col.name}
        {placed && (
          <button
            className="unrank"
            title="Some cards here are placed by hand. Give the whole column back to the automatic order."
            onClick={(e) => {
              e.stopPropagation();
              onClearRanks();
            }}
            aria-label={`Automatic order for ${col.name}`}
          >
            automatic
          </button>
        )}
        <span className={`count ${over ? "over" : ""}`} title={col.wip_limit != null ? `${cards.length} of a limit of ${col.wip_limit}` : `${cards.length} cards`}>
          {cards.length}
          {col.wip_limit != null ? ` / ${col.wip_limit}` : ""}
        </span>
      </h4>
      <SortableContext items={cards.map((c) => `${c.node_id}/${c.card.id}`)} strategy={verticalListSortingStrategy}>
        <div className="cards">
          {groups.map((g, gi) => {
            if (!g.state) return <div key={`flat-${gi}`}>{g.cards.map(one)}</div>;
            const key = `${col.id}:${g.state}`;
            const shut = collapsed.has(key);
            return (
              <div className={`grp tone-${STATE_TONE[g.state]}`} key={key}>
                <button className="grphead" onClick={() => onCollapse(key)} title={STATE_HELP[g.state]} aria-expanded={!shut}>
                  <span className="caret">{shut ? "›" : "⌄"}</span>
                  {STATE_LABEL[g.state]}
                  <span className="n">{g.cards.length}</span>
                </button>
                {!shut && <div className="grpcards">{g.cards.map(one)}</div>}
              </div>
            );
          })}
          {cards.length === 0 && <div className="empty">—</div>}
        </div>
      </SortableContext>
      <ColumnAdd columnName={col.name} onAdd={onAdd} onFull={onAddFull} />
    </div>
  );
}

export function Board({
  columns,
  cards,
  nodes,
  selected,
  onSelect,
  onMove,
  onReorder,
  onJump,
  onFilterNode,
  onAdd,
  onAddFull,
}: Props) {
  const [active, setActive] = useState<HubCard | null>(null);
  // The order the user just dropped, kept until the board comes back with it.
  const [dropped, setDropped] = useState<{ columnId: string; keys: string[]; from: HubCard[] } | null>(null);
  const pending = dropped && dropped.from === cards ? dropped : null;
  const [collapsed, setCollapsed] = useState<Set<string>>(() => {
    try {
      return new Set(JSON.parse(window.localStorage.getItem("kari.collapsedGroups") ?? "[]"));
    } catch {
      return new Set();
    }
  });
  const boardRef = useRef<HTMLDivElement>(null);
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }));
  const byId = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);
  const showNode = nodes.length > 1;
  const key = (c: HubCard) => `${c.node_id}/${c.card.id}`;

  const onCollapse = (k: string) =>
    setCollapsed((s) => {
      const next = new Set(s);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      try {
        window.localStorage.setItem("kari.collapsedGroups", JSON.stringify([...next]));
      } catch {
        // no storage: the choice holds for this run
      }
      return next;
    });

  const byColumn = useMemo(() => {
    const m = new Map<string, HubCard[]>();
    for (const col of columns) m.set(col.id, []);
    for (const c of cards) {
      const arr = m.get(c.column_id);
      if (arr) arr.push(c);
    }
    for (const arr of m.values()) arr.sort(sortCards);
    if (pending) {
      const arr = m.get(pending.columnId);
      if (arr && arr.length === pending.keys.length && arr.every((c) => pending.keys.includes(key(c)))) {
        arr.sort((a, b) => pending.keys.indexOf(key(a)) - pending.keys.indexOf(key(b)));
      }
    }
    return m;
  }, [columns, cards, pending]);

  // A mouse wheel sends up and down only. Over a column that cannot scroll
  // further, that motion moves the board sideways instead. A trackpad and
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

  // Drag the ground between the columns to pan the board, the way a map pans.
  // A mouse with no horizontal wheel needs it.
  useEffect(() => {
    const el = boardRef.current;
    if (!el) return;
    let from = 0;
    let at = 0;
    const onMove = (e: PointerEvent) => {
      el.scrollLeft = at - (e.clientX - from);
    };
    const stop = () => {
      el.classList.remove("panning");
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", stop);
    };
    const onDown = (e: PointerEvent) => {
      // Only the ground: a card, a button or a scrollable list keeps its own drag.
      const t = e.target as HTMLElement;
      if (e.button !== 0) return;
      if (t.closest(".card, button, input, textarea, select, .cards")) return;
      if (el.scrollWidth <= el.clientWidth) return;
      from = e.clientX;
      at = el.scrollLeft;
      el.classList.add("panning");
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", stop);
    };
    el.addEventListener("pointerdown", onDown);
    return () => {
      el.removeEventListener("pointerdown", onDown);
      stop();
    };
  }, []);

  function onDragStart(e: DragStartEvent) {
    setActive(cards.find((c) => key(c) === String(e.active.id)) ?? null);
  }

  function onDragEnd(e: DragEndEvent) {
    const from = active;
    setActive(null);
    if (!from || !e.over) return;
    const data = e.over.data.current as { type?: string; columnId?: string } | undefined;
    const overCard = cards.find((c) => key(c) === String(e.over!.id));
    const toColumn = data?.columnId ?? overCard?.column_id ?? String(e.over.id);

    // A drop on another column moves the card. Its rank goes back to automatic,
    // because the order of the old column says nothing about the new one.
    if (toColumn !== from.column_id) {
      onMove(from.node_id, from.card.id, toColumn);
      return;
    }
    if (!overCard || key(overCard) === key(from)) return;

    const column = (byColumn.get(from.column_id) ?? []).map(rankable);
    const plan = planReorder(column, key(from), key(overCard), from.node_id);
    if (!plan) return;

    setDropped({ columnId: from.column_id, keys: plan.order, from: cards });
    onReorder({ node: from.node_id, columnId: from.column_id, ranked: plan.ranked, unranked: plan.unranked });
  }

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCorners}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onDragCancel={() => setActive(null)}
    >
      <div className="board" ref={boardRef}>
        {columns.map((col) => (
          <ColumnView
            key={col.id}
            col={col}
            cards={byColumn.get(col.id) ?? []}
            nodes={byId}
            showNode={showNode}
            selected={selected}
            collapsed={collapsed}
            onCollapse={onCollapse}
            onSelect={onSelect}
            onJump={onJump}
            onFilterNode={onFilterNode}
            onAdd={(title) => onAdd(col.id, title)}
            onAddFull={() => onAddFull(col.id)}
            onClearRanks={() => {
              // One call per node: priorities live in each node's own store.
              const column = (byColumn.get(col.id) ?? []).map(rankable);
              for (const n of new Set(column.map((c) => c.node))) {
                const ids = clearRanks(column, n);
                if (ids.length > 0) onReorder({ node: n, columnId: col.id, ranked: [], unranked: ids });
              }
            }}
          />
        ))}
      </div>
      <DragOverlay dropAnimation={null}>
        {active ? <CardItem view={active} selected={false} showNode={showNode} overlay /> : null}
      </DragOverlay>
    </DndContext>
  );
}
