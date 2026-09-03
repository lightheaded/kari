import type { CardView, DerivedState, TokenTotals } from "./types";

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

/** Manual order first (a reorder writes priorities), then the urgency of the state, then recency. */
export function sortCards(a: CardView, b: CardView): number {
  if (a.card.priority !== b.card.priority) return b.card.priority - a.card.priority;
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

/** macOS WebKit autocorrects text fields unless told otherwise. Spread this on every free-text input. */
export const noAutoCorrect = { autoCorrect: "off", autoCapitalize: "off" } as const;
