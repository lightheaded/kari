import { describe, expect, test } from "bun:test";
import { createBoardLoader, isCard, withCard } from "./boardload";
import type { Card, Column, HubBoard } from "./types";

/** A fetch whose answers the test releases by hand, in any order. */
function manualFetch() {
  const pending: { resolve: (b: HubBoard) => void; reject: (e: unknown) => void }[] = [];
  const fetch = () =>
    new Promise<HubBoard>((resolve, reject) => {
      pending.push({ resolve, reject });
    });
  return { fetch, pending };
}

const board = (gen: string, cards: HubBoard["cards"] = []): HubBoard => ({
  columns: [],
  hub_id: "h",
  hub_name: "h",
  primary: true,
  nodes: [],
  cards,
  quotas: [],
  accounts: [],
  queues: [],
  proposals: [],
  generated_at: gen,
  scanning: false,
  herdr_connected: false,
  hooks_installed: false,
  hooks_port: 0,
});

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("createBoardLoader", () => {
  test("runs one fetch at a time, and the calls meanwhile share the next one", async () => {
    const { fetch, pending } = manualFetch();
    const seen: string[] = [];
    const l = createBoardLoader(fetch, { board: (b) => seen.push(b.generated_at), error: () => {} });
    l.load();
    l.load();
    l.load();
    expect(pending.length).toBe(1);
    pending[0].resolve(board("1"));
    await tick();
    expect(pending.length).toBe(2);
    pending[1].resolve(board("2"));
    await tick();
    expect(seen).toEqual(["1", "2"]);
    expect(pending.length).toBe(2);
  });

  test("a write drops the board that was in flight before it", async () => {
    const { fetch, pending } = manualFetch();
    const seen: string[] = [];
    const l = createBoardLoader(fetch, { board: (b) => seen.push(b.generated_at), error: () => {} });
    l.load();
    const after = l.wrote();
    pending[0].resolve(board("before the write"));
    await tick();
    expect(seen).toEqual([]);
    pending[1].resolve(board("after the write"));
    await after;
    expect(seen).toEqual(["after the write"]);
  });

  test("a failed fetch reports, and the next call fetches again", async () => {
    const { fetch, pending } = manualFetch();
    const errs: unknown[] = [];
    const l = createBoardLoader(fetch, { board: () => {}, error: (e) => errs.push(e) });
    const first = l.load();
    pending[0].reject("down");
    await first;
    expect(errs).toEqual(["down"]);
    l.load();
    expect(pending.length).toBe(2);
  });
});

const card = (over: Partial<Card> = {}): Card => ({
  id: "c1",
  kind: "task",
  title: "Write the notes",
  session_id: null,
  project_cwd: null,
  priority: 0,
  auto_run: false,
  run_prompt: null,
  permission_mode: null,
  model: null,
  mcp_servers: "default",
  estimate_weighted_tokens: null,
  manual_column: null,
  manual_lock_priority: null,
  tags: [],
  notes: null,
  archived: false,
  bg_job_id: null,
  last_job_state: null,
  last_job_at: null,
  scheduled: null,
  created_at: "2026-09-30T10:00:00Z",
  updated_at: "2026-09-30T10:00:00Z",
  done_at: null,
  ...over,
});

const col = (id: string, order: number, accepts: Column["accepts"]): Column => ({
  id,
  name: id,
  order,
  accepts,
  wip_limit: null,
  color: null,
  hidden: false,
});

describe("withCard", () => {
  const cols = [col("todo", 0, ["backlog"]), col("next", 1, ["ready"])];

  test("a new card lands in the column it was added in", () => {
    const b = withCard({ ...board("1"), columns: cols }, "n1", card(), { nodeName: "mac", columnId: "next", projectName: "kari" });
    expect(b.cards).toHaveLength(1);
    expect(b.cards[0].column_id).toBe("next");
    expect(b.cards[0].project_name).toBe("kari");
    expect(b.cards[0].node_id).toBe("n1");
  });

  test("a new card with no column lands where its state goes", () => {
    const b = withCard({ ...board("1"), columns: cols }, "n1", card(), { nodeName: "mac", columnId: null, projectName: null });
    expect(b.cards[0].column_id).toBe("todo");
  });

  test("a card on the board takes the new fields and keeps the rest", () => {
    const b1 = withCard({ ...board("1"), columns: cols }, "n1", card(), { nodeName: "mac", columnId: "next", projectName: "kari" });
    const b2 = withCard(b1, "n1", card({ notes: "a long paragraph" }));
    expect(b2.cards).toHaveLength(1);
    expect(b2.cards[0].card.notes).toBe("a long paragraph");
    expect(b2.cards[0].column_id).toBe("next");
  });

  test("a card that is not on the board, with nothing to place it, changes nothing", () => {
    const b = board("1");
    expect(withCard(b, "n1", card())).toBe(b);
  });
});

describe("isCard", () => {
  test("knows a card from other answers", () => {
    expect(isCard(card())).toBe(true);
    expect(isCard("Refreshing")).toBe(false);
    expect(isCard(null)).toBe(false);
    expect(isCard({ id: "x" })).toBe(false);
  });
});
