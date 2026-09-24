import { useCallback, useState } from "react";
import type { Picked } from "./components/Board";
import type { HubCard } from "./types";

/** A button the toast offers. Usually the reverse of what the user just did,
 *  which is why it says Undo unless the caller names another label. */
export interface Undo {
  /** What the next toast says after the button's action goes through. */
  done: string;
  run: () => Promise<unknown>;
  /** The button's text. Undo, when the caller names none. */
  label?: string;
}

export interface Toast {
  id: number;
  text: string;
  err?: boolean;
  /** A card to open when the toast is clicked. */
  card?: Picked | null;
  /** Milliseconds on screen. A notice from the engine holds long enough to read.
   *  A sticky toast has no end: Infinity. */
  ttl: number;
  undo?: Undo;
  /** The card waits for an answer. The toast stays until the card stops
   *  waiting or the user closes it. */
  sticky?: boolean;
}

export interface ToastOpts {
  err?: boolean;
  card?: Picked | null;
  ttl?: number;
  undo?: Undo;
  sticky?: boolean;
}

/** Report the result of one action, with an undo when the action has one.
 *  `card` names the card the action was about, so the toast can open it.
 *
 *  Resolves true when the action went through and false when it failed. The
 *  error reaches the user either way, as a toast. A caller that holds text the
 *  user typed must wait for true before it empties the box. */
export type Act = (fn: () => Promise<unknown>, ok?: string, undo?: Undo, card?: Picked | null) => Promise<boolean>;

/** The most toasts on screen at once. A run of notices drops the oldest,
 *  because a stack that fills the window is worse than a lost line. */
const MAX = 5;

/** A sticky toast younger than this stays, whatever the board says. The notice
 *  can arrive before the board that shows the card waiting. */
const SETTLE_GRACE_MS = 3000;

/** Keep the newest MAX toasts. Drop a plain toast before a sticky one, because
 *  a sticky toast is a session that cannot go on without the user. */
export function trim(list: Toast[], max = MAX): Toast[] {
  const out = [...list];
  while (out.length > max) {
    const i = out.findIndex((x) => !x.sticky);
    out.splice(i === -1 ? 0 : i, 1);
  }
  return out;
}

/** True for a card of `cards` that waits for an answer from the user. */
export function waitsOn(cards: HubCard[]): (card: Picked) => boolean {
  return (p) =>
    cards.some(
      (c) =>
        c.node_id === p.node && c.card.id === p.id && (c.state === "needs_approval" || c.state === "needs_decision"),
    );
}

/** Drop each sticky toast whose card no longer waits for the user. */
export function settled(list: Toast[], waits: (card: Picked) => boolean, now = Date.now()): Toast[] {
  return list.filter((x) => !x.sticky || !x.card || now - x.id < SETTLE_GRACE_MS || waits(x.card));
}

/** How long a toast stays, when the caller names no time. A toast that offers
 *  two buttons takes the longer of the two lives, because the user must read
 *  both before either one goes away. */
function lifeOf(o: ToastOpts): number {
  if (o.err) return 9000;
  return Math.max(o.undo ? 8000 : 0, o.card ? 10000 : 0, 4000);
}

/** The toast stack of one screen. */
export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const drop = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);
  const clear = useCallback(() => setToasts([]), []);
  const toast = useCallback((text: string, o: ToastOpts = {}) => {
    const id = Date.now() + Math.random();
    const ttl = o.sticky ? Infinity : (o.ttl ?? lifeOf(o));
    setToasts((t) => trim([...t, { id, text, err: o.err, card: o.card, undo: o.undo, ttl, sticky: o.sticky }]));
  }, []);
  /** Drop the sticky toasts of cards that stopped waiting. Call it with each new board. */
  const settle = useCallback((waits: (card: Picked) => boolean) => {
    setToasts((t) => {
      const next = settled(t, waits);
      return next.length === t.length ? t : next;
    });
  }, []);
  return { toasts, toast, drop, clear, settle };
}
