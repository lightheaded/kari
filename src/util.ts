import type {
  AccountQuota,
  CardView,
  DerivedState,
  NodeStatus,
  QuotaSample,
  QuotaWindow,
  ScheduleWhen,
  TokenTotals,
} from "./types";

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

/** The 5-hour rate-limit window, in milliseconds. */
export const FIVE_HOUR_MS = 5 * 3600_000;
/** kari books a run this long after the reset, so a run never meets the old
 *  window. The node applies the same margin: see `planner::RESET_MARGIN_MINUTES`. */
export const RESET_MARGIN_MS = 2 * 60_000;

/** The next reset of a window, from a sample that can be older than it.
 *  A machine that slept reports a reset that has passed, and every window
 *  after it is the same length, so step forward until the time is ahead. */
export function nextReset(
  resetsAt: string | null | undefined,
  lengthMs: number,
  now = Date.now(),
): number | null {
  if (!resetsAt) return null;
  let t = new Date(resetsAt).getTime();
  if (Number.isNaN(t)) return null;
  let guard = 0;
  while (t <= now && guard < 400) {
    t += lengthMs;
    guard += 1;
  }
  return t > now ? t : null;
}

/** The time each choice would book, for the labels on the buttons. The node
 *  resolves the real time from its own sample, so this is a preview. */
export function schedulePreview(
  quota: QuotaSample | null | undefined,
  now = Date.now(),
): Partial<Record<ScheduleWhen, string>> {
  const five = nextReset(quota?.five_hour?.resets_at, FIVE_HOUR_MS, now);
  const week = nextReset(quota?.seven_day?.resets_at, 7 * 24 * 3600_000, now);
  const out: Partial<Record<ScheduleWhen, string>> = {};
  if (five !== null) {
    out.next_reset = new Date(five + RESET_MARGIN_MS).toISOString();
    out.following_cycle = new Date(five + FIVE_HOUR_MS + RESET_MARGIN_MS).toISOString();
  }
  if (week !== null) out.weekly_reset = new Date(week + RESET_MARGIN_MS).toISOString();
  return out;
}

/** When a window resets, short enough for the strip. Every window says
 *  something, because a blank box beside a bar reads as a missing number.
 *
 *  A window kari has no reading for says "no data". A window with a reading
 *  but no reset time has not started: Claude Code opens it at the first
 *  message after the last reset, and only then names the time it ends. */
export function resetIn(w: QuotaWindow | null | undefined, now = Date.now()): string {
  if (!w) return "no data";
  if (!w.resets_at) return "not started";
  return untilTime(w.resets_at, now);
}

/** The same fact in a sentence, for the tooltip of a meter. */
export function resetTitle(label: string, w: QuotaWindow | null | undefined, now = Date.now()): string {
  if (!w) return `${label}: kari has no reading for this window.`;
  const used = `${label}: ${Math.max(0, Math.min(100, w.used_percentage)).toFixed(0)}% used.`;
  if (!w.resets_at) {
    return `${used} The window is not started. It opens at the next message, and the reset time follows.`;
  }
  return `${used} Resets ${clock(w.resets_at)}, in ${untilTime(w.resets_at, now)}.`;
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

/** True when a composer can empty itself after a send.
 *
 *  Two conditions, and both must hold. The send went through: a failed send
 *  keeps the text, so a retry costs one tap and not the message. And the box
 *  still holds the text that went: anything the user typed while the send was
 *  in flight is a new message, and it stays.
 *
 *  `box` is null when the box is closed, which is how the phone card holds it. */
export function clearsBox(box: string | null, sent: string, ok: boolean): boolean {
  return ok && box !== null && box.trim() === sent;
}

/** How many colours the board gives out to nodes. */
const HUES = 8;

/** The colour class of one node.
 *
 *  A machine keeps one colour on every screen: the chip on its cards, the dot
 *  in the filter bar, and the row in the quota strip. The colour comes from
 *  the node id, so it survives a restart, and two hubs that see the same node
 *  paint it the same. Nothing stores it, and nobody picks it. */
export function nodeHue(nodeId: string): string {
  let h = 0;
  for (let i = 0; i < nodeId.length; i++) h = (Math.imul(h, 31) + nodeId.charCodeAt(i)) >>> 0;
  return `hue-${h % HUES}`;
}

/** Class for the status dot of a node: accent when online, muted when
 *  offline, hollow when off. The node colour rides along, for the places
 *  whose question is which machine and not whether it answers. */
export function nodeDot(n: NodeStatus): string {
  if (!n.enabled) return "dot disabled";
  return `dot ${n.online ? "online" : "offline"} ${nodeHue(n.id)}`;
}

/** The account label of each node, for the screens that name the account.
 *
 *  The map is empty while the board spends one account, because a name that
 *  never changes tells the reader nothing and takes room on every card. Two
 *  accounts or more, and each node carries the name of the one that pays.
 *
 *  A quota row of a node whose account kari could not read is keyed on the
 *  node and labelled with the node name. Such a row names no account, so it
 *  is left out: a tag that reads `studio · studio` says the same thing twice. */
export function accountByNode(accounts: AccountQuota[]): Map<string, string> {
  const m = new Map<string, string>();
  if (new Set(accounts.map((a) => a.label)).size < 2) return m;
  for (const a of accounts) {
    if (a.key.startsWith("node:") && !a.account) continue;
    for (let i = 0; i < a.node_ids.length; i++) {
      if (a.label && a.label !== a.node_names[i]) m.set(a.node_ids[i], a.label);
    }
  }
  return m;
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

/** A one-line task, with the project tag read out of it. */
export interface TaggedTask {
  /** The title without the tag. This is what the card gets. */
  title: string;
  /** What the user typed after the `#`, or an empty string for no tag. */
  tag: string;
  /** Where the task goes. The tag wins over the filters when it matches. */
  target: AddTarget;
  /** True when a tag is there and no project answers to it. */
  unknown: boolean;
}

/** A project tag: a `#` at the start of a word, then the name. */
const TAG = /(^|\s)#([A-Za-z0-9][\w.\-/]*)/g;

/** The last part of a directory path. A project goes by that name too. */
function baseName(cwd: string): string {
  return cwd.split("/").filter(Boolean).pop() ?? cwd;
}

/**
 * Find the project that a tag names.
 *
 * The tag is matched against the project name and against the last part of the
 * directory. An exact match wins, then a match on the start of the name, then
 * the best fuzzy match. Two projects with the same score keep the first of the
 * list, which is sorted by name, so the answer holds between keystrokes.
 *
 * A node filter limits the search to that node. Without a filter, the match
 * names the node as well, because a path lives on one machine.
 */
export function matchProject(tag: string, projects: [string, FilterProject][], node: string): FilterProject | null {
  if (!tag) return null;
  const open = projects.map(([, p]) => p).filter((p) => !node || p.node === node);
  const t = tag.toLowerCase();
  const names = (p: FilterProject) => [p.name.toLowerCase(), baseName(p.cwd).toLowerCase()];
  const exact = open.find((p) => names(p).includes(t));
  if (exact) return exact;
  const starts = open.find((p) => names(p).some((n) => n.startsWith(t)));
  if (starts) return starts;
  let best: FilterProject | null = null;
  let score = 0;
  for (const p of open) {
    const s = Math.max(...names(p).map((n) => fuzzyScore(n, tag)));
    if (s > score) {
      best = p;
      score = s;
    }
  }
  return best;
}

/**
 * Read a `#project` tag out of a one-line task, and say where the task goes.
 *
 * The quick add box holds one line and no picker. A tag lets that line name the
 * project, as a click on the project filter does. The tag is then cut out of
 * the title, so the card reads as the user meant it.
 *
 * A line can hold more than one `#word`. The first word that names a project
 * wins, and the others stay in the title. A word of digits alone, such as
 * `#1234`, is an issue number and is never read as a tag. A tag that names no
 * project also stays in the title: a task must not lose text because the board
 * knows no such name. The caller reads `unknown` and says that nothing matched.
 *
 * `filters` says where the task goes when no tag names a project: the project
 * filter, the node filter and the last project used, as `addTarget` reads them.
 */
export function taggedTask(
  raw: string,
  projects: [string, FilterProject][],
  filters: { project: string; node: string; last: string },
): TaggedTask {
  const fallback = addTarget(projects, filters.project, filters.node, filters.last);
  const plain = { title: raw.trim(), tag: "", target: fallback, unknown: false };
  const tags = [...raw.matchAll(TAG)].filter((m) => !/^\d+$/.test(m[2]));
  if (tags.length === 0) return plain;
  const hit = tags.map((m) => ({ m, p: matchProject(m[2], projects, filters.node) })).find((x) => x.p);
  if (!hit) return { ...plain, tag: tags[0][2], unknown: true };
  const { m, p } = hit;
  // Cut the tag out, and close the gap that it leaves inside a sentence.
  const cut = (raw.slice(0, m.index) + m[1] + raw.slice(m.index + m[0].length)).replace(/\s+/g, " ").trim();
  // A line that holds the tag and nothing else keeps it. An empty title is
  // worse than a title that repeats the project.
  return { title: cut || raw.trim(), tag: m[2], target: { node: p!.node, cwd: p!.cwd, name: p!.name }, unknown: false };
}
