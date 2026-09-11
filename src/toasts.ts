import { useCallback, useState } from "react";
import type { Picked } from "./components/Board";

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
  /** Milliseconds on screen. A notice from the engine holds long enough to read. */
  ttl: number;
  undo?: Undo;
}

export interface ToastOpts {
  err?: boolean;
  card?: Picked | null;
  ttl?: number;
  undo?: Undo;
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
    setToasts((t) => [...t, { id, text, err: o.err, card: o.card, undo: o.undo, ttl: o.ttl ?? lifeOf(o) }].slice(-MAX));
  }, []);
  return { toasts, toast, drop, clear };
}
