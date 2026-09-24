import { useEffect, useSyncExternalStore } from "react";

/** A prompt the user sent from this screen that the transcript does not show
 *  yet.
 *
 *  A send to a running session takes about a second, because kari waits for
 *  the receipt of the inbox. After that the prompt waits in the queue of the
 *  session, and a busy session reads it only after its current turn. The
 *  transcript shows the prompt at that point and not before, so without this
 *  the message the user sent is gone from the screen for that whole time. */
export interface PendingSend {
  id: number;
  node: string;
  card: string;
  text: string;
  /** When the user pressed send, in the clock of this screen. */
  at: number;
  /** `sending` until the node answers. `sent` when the node took it, and
   *  `failed` when the node refused it and the box did not take it back. */
  state: "sending" | "sent" | "failed";
  /** What the node said about the send, or the error. */
  note?: string;
}

/** How long a sent prompt waits for the transcript before it leaves the
 *  screen. A held message, or a session that was stopped, never shows up. */
const FORGET_MS = 60 * 60 * 1000;
/** How far the clock of a node and the clock of this screen can differ. */
const SKEW_MS = 2 * 60 * 1000;

let items: PendingSend[] = [];
const listeners = new Set<() => void>();

function set(next: PendingSend[]) {
  items = next;
  for (const l of listeners) l();
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

function update(id: number, p: Partial<PendingSend>) {
  set(items.map((x) => (x.id === id ? { ...x, ...p } : x)));
}

/** Every entry, on every card. */
export function pendingSends(): PendingSend[] {
  return items;
}

/** Take one entry off the screen. */
export function dismissSend(id: number) {
  set(items.filter((x) => x.id !== id));
}

/** Send a prompt and show it on the screen from the moment it leaves.
 *
 *  `restore` gets the text back when the send fails. It answers true when the
 *  prompt box took the text, and the entry then leaves the screen, because the
 *  text is back where the user can send it again. When the box holds new text,
 *  the failed entry stays with its error, so the prompt is not lost. The
 *  promise settles as `fn` settles. */
export async function trackSend<T>(node: string, card: string, text: string, fn: () => Promise<T>, restore: (text: string) => boolean): Promise<T> {
  const id = Date.now() + Math.random();
  set([...items, { id, node, card, text, at: Date.now(), state: "sending" }]);
  try {
    const r = await fn();
    update(id, { state: "sent", note: typeof r === "string" ? r : undefined });
    return r;
  } catch (e) {
    if (restore(text)) dismissSend(id);
    else update(id, { state: "failed", note: String(e) });
    throw e;
  }
}

/** A stable empty list, so a view with no transcript does not make a new one
 *  on every render. */
export const NO_TURNS: { role: string; text: string; at: string | null }[] = [];

/** The text of a turn holds the prompt. A peer turn wraps the prompt in an
 *  envelope, and a card with attachments adds the list of files after it. */
function holds(turn: string, sent: string): boolean {
  return turn.includes(sent.trim());
}

/** The prompts this screen sent to one card that the transcript does not show
 *  yet, oldest first.
 *
 *  `seen` is what the screen knows of the transcript: the loaded turns, or
 *  only the last prompt. A prompt leaves the list as soon as a turn from the
 *  user or a peer holds its text, at the time of the send or later. */
export function usePendingSends(node: string, card: string, seen: { role: string; text: string; at: string | null }[]): PendingSend[] {
  const all = useSyncExternalStore(subscribe, () => items);

  useEffect(() => {
    const now = Date.now();
    const done = all.filter((p) => {
      if (p.node !== node || p.card !== card) return false;
      if (p.state === "sent" && now - p.at > FORGET_MS) return true;
      if (p.state === "sending") return false;
      return seen.some((t) => {
        if (t.role !== "user" && t.role !== "peer") return false;
        if (t.at && Date.parse(t.at) < p.at - SKEW_MS) return false;
        return holds(t.text, p.text);
      });
    });
    if (done.length > 0) set(items.filter((x) => !done.includes(x)));
  }, [all, node, card, seen]);

  return all.filter((p) => p.node === node && p.card === card);
}

/** Whether a send to this card is still on its way to the node. */
export function isSending(list: PendingSend[]): boolean {
  return list.some((p) => p.state === "sending");
}
