import { describe, expect, test } from "bun:test";
import { settled, trim, waitsOn, type Toast } from "./toasts";
import type { HubCard } from "./types";

function t(id: number, sticky = false, card = "c1"): Toast {
  return { id, text: `t${id}`, ttl: sticky ? Infinity : 4000, sticky, card: { node: "local", id: card } };
}

describe("sticky toasts", () => {
  test("a run of plain toasts never pushes out a sticky one", () => {
    const list = [t(1, true), t(2), t(3), t(4), t(5), t(6)];
    expect(trim(list, 5).map((x) => x.id)).toEqual([1, 3, 4, 5, 6]);
  });

  test("a sticky toast goes when its card stops waiting, after the grace time", () => {
    const cards = [{ node_id: "local", card: { id: "c1" }, state: "working" }] as unknown as HubCard[];
    const list = [t(1000, true), t(1000)];
    // Too young: the board can still be the one from before the notice.
    expect(settled(list, waitsOn(cards), 2000)).toHaveLength(2);
    expect(settled(list, waitsOn(cards), 10000).map((x) => x.sticky)).toEqual([false]);
    const waiting = [{ node_id: "local", card: { id: "c1" }, state: "needs_approval" }] as unknown as HubCard[];
    expect(settled(list, waitsOn(waiting), 10000)).toHaveLength(2);
  });
});
