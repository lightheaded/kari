import type { CardView, DerivedState, NodeStatus, TokenTotals } from "./types";

export function relTime(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "—";
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return "—";
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h}h`;
  const d = Math.round(h / 24);
  return `${d}d`;
}

export function untilTime(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "";
  const t = new Date(iso).getTime();
  const s = Math.max(0, Math.round((t - now) / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h >= 48) return `${Math.round(h / 24)}d`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function clock(iso: string | null | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  const sameDay = d.toDateString() === new Date().toDateString();
  const hm = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (sameDay) return hm;
  return `${d.toLocaleDateString([], { weekday: "short" })} ${hm}`;
}

export function weighted(t: TokenTotals | undefined | null): number {
  if (!t) return 0;
  return t.input + t.cache_write * 1.25 + t.cache_read * 0.1 + t.output * 5;
}

export function fmtM(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 0 : 1)}M`;
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`;
  return `${Math.round(n)}`;
}

export function fmtPct(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return "—";
  if (n >= 10) return `${Math.round(n)}%`;
  if (n >= 1) return `${n.toFixed(1)}%`;
  return `${n.toFixed(2)}%`;
}

export const STATE_TONE: Record<DerivedState, "green" | "amber" | "rust" | "slate" | "neutral"> = {
  backlog: "neutral",
  ready: "green",
  working: "green",
  my_turn: "slate",
  needs_decision: "amber",
  needs_approval: "rust",
  waiting_on_others: "slate",
  validate: "green",
  done: "neutral",
  stale: "neutral",
  unknown: "neutral",
};

export function statePriority(s: DerivedState): number {
  return (
    {
      needs_approval: 90,
      needs_decision: 85,
      working: 70,
      validate: 60,
      my_turn: 50,
      waiting_on_others: 40,
      done: 30,
      ready: 22,
      backlog: 20,
      stale: 10,
      unknown: 0,
    } as Record<DerivedState, number>
  )[s];
}

/** A card the user placed by hand carries a priority. Priority 0 means automatic. */
export function isRanked(c: CardView): boolean {
  return c.card.priority !== 0;
}

/** Ranked cards hold the order the user gave them. Unranked cards follow, by
 *  the urgency of their state and then by recency. A card nobody dragged
 *  therefore never jumps above one that was placed by hand. */
export function sortCards(a: CardView, b: CardView): number {
  const ra = isRanked(a);
  const rb = isRanked(b);
  if (ra !== rb) return ra ? -1 : 1;
  if (ra && rb) return b.card.priority - a.card.priority;
  const pa = statePriority(a.state) + (a.live ? 5 : 0);
  const pb = statePriority(b.state) + (b.live ? 5 : 0);
  if (pa !== pb) return pb - pa;
  const ta = a.last_activity_at ? new Date(a.last_activity_at).getTime() : 0;
  const tb = b.last_activity_at ? new Date(b.last_activity_at).getTime() : 0;
  return tb - ta;
}

export function shortId(id: string | null | undefined): string {
  return id ? id.slice(0, 8) : "";
}

/** Class for the status dot of a node: accent when online, muted when offline, hollow when off. */
export function nodeDot(n: NodeStatus): string {
  if (!n.enabled) return "dot disabled";
  return n.online ? "dot online" : "dot offline";
}

/** macOS text fields in kari must behave like a native app: no autofill list,
 *  no autocorrect, no first-letter capital. Spread this on every text input.
 *  A long prose field adds `spellCheck` of its own. */
export const noAutoFill = {
  autoComplete: "off",
  autoCorrect: "off",
  autoCapitalize: "off",
  spellCheck: false,
} as const;

/** The same, for a field that holds prose the user wants checked. */
export const proseField = {
  autoComplete: "off",
  autoCorrect: "off",
  autoCapitalize: "off",
  spellCheck: true,
} as const;

/** Score a fuzzy match of `needle` against `hay`. Higher is better, 0 is no
 *  match. Every character of the needle must appear in order. A match at a word
 *  start, and a run of neighbouring characters, both score higher. */
export function fuzzyScore(hay: string, needle: string): number {
  if (!needle) return 1;
  const h = hay.toLowerCase();
  const n = needle.toLowerCase();
  let score = 0;
  let at = 0;
  let run = 0;
  for (const ch of n) {
    const i = h.indexOf(ch, at);
    if (i < 0) return 0;
    run = i === at ? run + 1 : 0;
    const wordStart = i === 0 || /[^a-z0-9]/.test(h[i - 1]);
    score += 1 + run * 2 + (wordStart ? 3 : 0);
    at = i + 1;
  }
  // A short haystack that matches is a better answer than a long one.
  return score + Math.max(0, 20 - h.length) / 20;
}

/** What one card in a column needs, to work out a new manual order. */
export interface Rankable {
  /** Unique across the board: node and card together. */
  key: string;
  /** The node that stores this card's priority. */
  node: string;
  /** The card id, as the node knows it. */
  id: string;
  /** Non-zero when the user placed this card by hand. */
  priority: number;
}

/** The new order of one column after a drop, and the two priority lists the
 *  node must store.
 *
 *  The rule: every card down to the lowest hand-placed card is hand-placed, in
 *  the order shown. Everything below that keeps priority 0 and so keeps the
 *  automatic order. A card the user never dragged therefore never jumps above
 *  one that was placed.
 *
 *  Priorities live in the store of one node, so only the cards of the dragged
 *  card's node go into the two lists. */
export function planReorder<T extends Rankable>(
  column: T[],
  fromKey: string,
  overKey: string,
  node: string,
): { order: string[]; ranked: string[]; unranked: string[] } | null {
  const list = [...column];
  const oldAt = list.findIndex((c) => c.key === fromKey);
  const newAt = list.findIndex((c) => c.key === overKey);
  if (oldAt < 0 || newAt < 0 || oldAt === newAt) return null;
  list.splice(newAt, 0, ...list.splice(oldAt, 1));

  // The run reaches the lowest placed card, so a drop only ever adds to it.
  // `clearRanks` is the way back to the automatic order.
  let last = newAt;
  for (let i = 0; i < list.length; i++) if (list[i].priority !== 0) last = Math.max(last, i);
  const placed = new Set(list.slice(0, last + 1).map((c) => c.key));
  const mine = list.filter((c) => c.node === node);
  return {
    order: list.map((c) => c.key),
    ranked: mine.filter((c) => placed.has(c.key)).map((c) => c.id),
    unranked: mine.filter((c) => !placed.has(c.key) && c.priority !== 0).map((c) => c.id),
  };
}

/** Every placed card of one node in a column, so a reset can clear them.
 *  The result goes to `reorder_cards` as the `unranked` list. */
export function clearRanks<T extends Rankable>(column: T[], node: string): string[] {
  return column.filter((c) => c.node === node && c.priority !== 0).map((c) => c.id);
}

/** One project directory on one node, as the board's filter list holds it. */
export interface FilterProject {
  node: string;
  cwd: string;
  name: string;
}

/** Where a new task goes. */
export interface AddTarget {
  /** The node that gets the card. Empty means no filter names one. */
  node: string;
  /** The project directory, or null when nothing names one. */
  cwd: string | null;
  /** The name to show for that directory, or null. */
  name: string | null;
}

/**
 * Read the node and the project a new task must go to out of the filters.
 *
 * The project filter names a node as well as a directory, so both must come
 * from the same entry. A project on another node once sent the card to the
 * local one, where its path does not exist.
 *
 * `projects` is the filter list, keyed the way the filter values are. `project`
 * is the chosen project filter, `node` the chosen node filter, and `last` the
 * project the last task went to.
 */
export function addTarget(
  projects: [string, FilterProject][],
  project: string,
  node: string,
  last: string,
): AddTarget {
  const pick = (key: string) => {
    if (!key) return null;
    const p = projects.find(([k]) => k === key)?.[1];
    // A node filter wins over a remembered project on another node.
    return p && (!node || p.node === node) ? p : null;
  };
  const p = pick(project) ?? pick(last);
  return { node: p?.node ?? node, cwd: p?.cwd ?? null, name: p?.name ?? null };
}
