import { useCallback, useMemo, useState } from "react";
import type { Card, Column, DerivedState, HubBoard, HubCard } from "./types";

/** What a board fetch does with its answer. */
export interface BoardSink {
  board: (b: HubBoard) => void;
  error: (e: unknown) => void;
}

export interface BoardLoader {
  /** Fetch the board. A call while a fetch runs waits for one more fetch
   *  after it, and every caller in that wait shares it. The promise settles
   *  when a board that started after the call is on screen, or failed. */
  load: () => Promise<void>;
  /** A write went through. A fetch that started before the write carries the
   *  board as it was, so its answer is dropped, and a fresh fetch follows. */
  wrote: () => Promise<void>;
}

/**
 * The board, one fetch at a time, and never older than what is on screen.
 *
 * A board fetch takes long on a busy machine: this machine's scan, plus the
 * server's board over the network. Every hook event, every write and the poll
 * each asked for a fetch, and the fetches ran side by side. They could answer
 * out of order, so a board from before a write came in after the board with
 * the write. A new card left the column for some seconds, and the notes just
 * typed into a card went back to the old text until the next fetch.
 *
 * So one fetch runs at a time, the calls that come in meanwhile share the next
 * one, and a write makes the fetch in flight stale.
 */
export function createBoardLoader(fetch: () => Promise<HubBoard>, sink: BoardSink): BoardLoader {
  let running: Promise<void> | null = null;
  let queued: Promise<void> | null = null;
  let stale = false;

  const start = (): Promise<void> => {
    stale = false;
    running = (async () => {
      try {
        const b = await fetch();
        if (!stale) sink.board(b);
      } catch (e) {
        if (!stale) sink.error(e);
      } finally {
        running = null;
      }
    })();
    return running;
  };

  const load = (): Promise<void> => {
    if (queued) return queued;
    if (running) {
      queued = running.then(() => {
        queued = null;
        return start();
      });
      return queued;
    }
    return start();
  };

  const wrote = (): Promise<void> => {
    if (running) stale = true;
    return load();
  };

  return { load, wrote };
}

/** What the board needs to show a card that the node made a moment ago. */
export interface NewCardInfo {
  nodeName: string;
  /** The column the user added the card in, when there was one. */
  columnId: string | null;
  projectName: string | null;
}

/** The column a state lands in, as the node picks it: the first open column
 *  that takes the state, else the first one. */
function columnOf(columns: Column[], state: DerivedState): string {
  const sorted = [...columns].sort((a, b) => a.order - b.order);
  return (sorted.find((c) => !c.hidden && c.accepts.includes(state)) ?? sorted[0])?.id ?? "";
}

/**
 * Put a card that a write answered with on the board, before the next fetch.
 *
 * A card that is on the board takes the new fields. A card that is not there
 * yet is added when `fresh` says where it goes. The next fetch replaces both
 * with what the node derives, so this only has to be close.
 */
export function withCard(b: HubBoard, node: string, card: Card, fresh?: NewCardInfo): HubBoard {
  const i = b.cards.findIndex((c) => c.node_id === node && c.card.id === card.id);
  if (i >= 0) {
    const old = b.cards[i];
    const cards = b.cards.slice();
    cards[i] = { ...old, card, title: card.title || old.title };
    return { ...b, cards };
  }
  if (!fresh) return b;
  const state: DerivedState = card.auto_run ? "ready" : "backlog";
  const view: HubCard = {
    node_id: node,
    node_name: fresh.nodeName,
    card,
    title: card.title ?? "",
    state,
    column_id: card.manual_column ?? fresh.columnId ?? columnOf(b.columns, state),
    locked: card.manual_column !== null,
    project_name: fresh.projectName,
    session: null,
    live: null,
    bg_job: null,
    herdr: null,
    summary: null,
    hooks: null,
    estimate: null,
    last_activity_at: card.created_at,
    reason: "",
    attachments: [],
  };
  return { ...b, cards: [...b.cards, view] };
}

/** True when a value is a card, as a write on a card answers. */
export function isCard(v: unknown): v is Card {
  return (
    typeof v === "object" &&
    v !== null &&
    typeof (v as Card).id === "string" &&
    ((v as Card).kind === "task" || (v as Card).kind === "session") &&
    typeof (v as Card).updated_at === "string"
  );
}

/** The board of a window: fetched in order, and written ahead of the node
 *  when a write answers with a card. `failed` counts the fetches that failed
 *  since the last one that answered. */
export function useBoard(fetch: () => Promise<HubBoard>) {
  const [board, setBoard] = useState<HubBoard | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [failed, setFailed] = useState(0);
  const loader = useMemo(
    () =>
      createBoardLoader(fetch, {
        board: (b) => {
          setBoard(b);
          setError(null);
          setFailed(0);
        },
        error: (e) => {
          setError(String(e));
          setFailed((n) => n + 1);
        },
      }),
    [fetch],
  );
  const put = useCallback(
    (node: string, card: Card, fresh?: NewCardInfo) => setBoard((b) => (b ? withCard(b, node, card, fresh) : b)),
    [],
  );
  return { board, error, failed, load: loader.load, wrote: loader.wrote, put };
}
