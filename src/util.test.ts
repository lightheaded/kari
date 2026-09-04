import { describe, expect, test } from "bun:test";
import { fuzzyScore, planReorder, sortCards, type Rankable } from "./util";
import type { CardView, DerivedState } from "./types";

/** One card in a column, for the reorder tests. */
const c = (key: string, priority = 0, node = "local"): Rankable => ({ key, node, id: key, priority });

describe("planReorder", () => {
  test("a drop places the card and only what sits above it", () => {
    // Nothing is placed yet. D lands second, so A and D are placed. B and C
    // are below it and keep the automatic order.
    const col = [c("a"), c("b"), c("c"), c("d")];
    const p = planReorder(col, "d", "b", "local")!;
    expect(p.order).toEqual(["a", "d", "b", "c"]);
    expect(p.ranked).toEqual(["a", "d"]);
    expect(p.unranked).toEqual([]);
  });

  test("a drop at the top places that one card only", () => {
    const col = [c("a"), c("b"), c("c"), c("d")];
    const p = planReorder(col, "b", "a", "local")!;
    expect(p.order).toEqual(["b", "a", "c", "d"]);
    // B alone carries a priority. A, C and D still sort by themselves, and
    // they all sit below B because a placed card always comes first.
    expect(p.ranked).toEqual(["b"]);
    expect(p.unranked).toEqual([]);
  });

  test("a second drop keeps the first one placed", () => {
    // B is placed from the drop above. Dragging C over A must not lose B.
    const col = [c("b", 1), c("a"), c("c"), c("d")];
    const p = planReorder(col, "c", "a", "local")!;
    expect(p.order).toEqual(["b", "c", "a", "d"]);
    expect(p.ranked).toEqual(["b", "c"]);
    expect(p.unranked).toEqual([]);
  });

  test("a placed card below the drop keeps the run open to it", () => {
    // C carries a priority already, so the placed run reaches down to it and
    // B, which sits between them, is placed as well.
    const col = [c("a"), c("b"), c("c", 5), c("d")];
    const p = planReorder(col, "d", "a", "local")!;
    expect(p.order).toEqual(["d", "a", "b", "c"]);
    expect(p.ranked).toEqual(["d", "a", "b", "c"]);
    expect(p.unranked).toEqual([]);
  });

  test("a card dragged to the foot of the placed run stays placed, last", () => {
    const col = [c("a", 3), c("b", 2), c("c"), c("d")];
    const p = planReorder(col, "a", "d", "local")!;
    expect(p.order).toEqual(["b", "c", "d", "a"]);
    expect(p.ranked).toEqual(["b", "c", "d", "a"]);
    expect(p.unranked).toEqual([]);
  });

  test("a placed card dragged below the run gives its priority back", () => {
    // A is the only placed card. Dropping it under D ends the run at D, and
    // nothing above it was placed, so A goes back to the automatic order.
    const col = [c("a", 3), c("b"), c("c"), c("d")];
    const p = planReorder(col, "d", "b", "local")!;
    expect(p.order).toEqual(["a", "d", "b", "c"]);
    expect(p.ranked).toEqual(["a", "d"]);
    expect(p.unranked).toEqual([]);
  });

  test("only the cards of the dragged card's node get a priority", () => {
    // Priorities live in one node's store, so a card of another node never
    // reaches either list, even inside the placed run.
    const col = [c("a"), c("x", 0, "lab"), c("b", 4), c("d")];
    const p = planReorder(col, "d", "a", "local")!;
    expect(p.order).toEqual(["d", "a", "x", "b"]);
    expect(p.ranked).toEqual(["d", "a", "b"]);
    expect(p.unranked).toEqual([]);
  });

  test("a drop never sends a card back to automatic", () => {
    // The placed run always reaches the lowest placed card, so a drop can only
    // add to the run. The column header has the reset for the other direction.
    const col = [c("a"), c("b", 7), c("c")];
    const p = planReorder(col, "c", "a", "local")!;
    expect(p.order).toEqual(["c", "a", "b"]);
    expect(p.ranked).toEqual(["c", "a", "b"]);
    expect(p.unranked).toEqual([]);
  });

  test("a drop on itself, or on a card that is not there, changes nothing", () => {
    const col = [c("a"), c("b")];
    expect(planReorder(col, "a", "a", "local")).toBeNull();
    expect(planReorder(col, "a", "zz", "local")).toBeNull();
  });
});

/** The smallest card the sort reads. */
const card = (id: string, state: DerivedState, priority: number, at: string) =>
  ({
    card: { id, priority },
    state,
    live: null,
    last_activity_at: at,
  }) as unknown as CardView;

describe("sortCards", () => {
  test("a placed card comes before every automatic card", () => {
    const placed = card("p", "backlog", 4, "2026-09-01T00:00:00Z");
    const urgent = card("u", "needs_approval", 0, "2026-09-04T00:00:00Z");
    expect([urgent, placed].sort(sortCards).map((x) => x.card.id)).toEqual(["p", "u"]);
  });

  test("two placed cards follow their priority, high first", () => {
    const a = card("a", "backlog", 2, "2026-09-01T00:00:00Z");
    const b = card("b", "backlog", 7, "2026-09-01T00:00:00Z");
    expect([a, b].sort(sortCards).map((x) => x.card.id)).toEqual(["b", "a"]);
  });

  test("two automatic cards follow the urgency of their state", () => {
    const quiet = card("q", "backlog", 0, "2026-09-04T00:00:00Z");
    const urgent = card("u", "needs_approval", 0, "2026-09-01T00:00:00Z");
    expect([quiet, urgent].sort(sortCards).map((x) => x.card.id)).toEqual(["u", "q"]);
  });

  test("two automatic cards of one state follow recency", () => {
    const old = card("o", "backlog", 0, "2026-09-01T00:00:00Z");
    const fresh = card("f", "backlog", 0, "2026-09-04T00:00:00Z");
    expect([old, fresh].sort(sortCards).map((x) => x.card.id)).toEqual(["f", "o"]);
  });
});

describe("fuzzyScore", () => {
  test("every character must appear, in order", () => {
    expect(fuzzyScore("storefront-web", "stor")).toBeGreaterThan(0);
    expect(fuzzyScore("storefront-web", "strf")).toBeGreaterThan(0);
    expect(fuzzyScore("storefront-web", "zzz")).toBe(0);
    expect(fuzzyScore("storefront-web", "rots")).toBe(0);
  });

  test("a run of neighbours beats characters that are far apart", () => {
    expect(fuzzyScore("storefront-web", "stor")).toBeGreaterThan(fuzzyScore("storefront-web", "swb"));
  });

  test("an empty query matches everything", () => {
    expect(fuzzyScore("anything", "")).toBe(1);
  });

  test("a match at a word start scores higher", () => {
    // "web" starts a word in "storefront-web" but not in "webbing-store".
    expect(fuzzyScore("storefront-web", "web")).toBeGreaterThan(fuzzyScore("astorefrontweb", "web"));
  });
});
