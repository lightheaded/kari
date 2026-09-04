import { useCallback, useState } from "react";
import type { Picked } from "./components/Board";

/** The reverse of an action the user just took. The toast that reports the
 *  action offers it as an Undo button. */
export interface Undo {
  /** What the next toast says after the reversal goes through. */
  done: string;
  run: () => Promise<unknown>;
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

/** Report the result of one action, with an undo when the action has one. */
export type Act = (fn: () => Promise<unknown>, ok?: string, undo?: Undo) => Promise<void>;

/** The most toasts on screen at once. A run of notices drops the oldest,
 *  because a stack that fills the window is worse than a lost line. */
const MAX = 5;

/** How long a toast stays, when the caller names no time. */
function lifeOf(o: ToastOpts): number {
  if (o.err) return 9000;
  if (o.undo) return 8000;
  if (o.card) return 10000;
  return 4000;
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
