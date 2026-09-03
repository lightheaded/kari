import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { BoardView, Calibration, Card, CardPatch, Column, JobLogEntry, NewTask, Proposal, QuotaSample, Settings, Summary } from "./types";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Browser preview without Tauri: read `fixtures/dev-board.json`, exported by `cargo run -p kari-core --example board -- --json`. */
async function devBoard(): Promise<BoardView> {
  const r = await fetch("/dev-board.json");
  if (!r.ok) throw new Error("no fixtures/dev-board.json; run the board example with --json");
  return r.json();
}

export const api = {
  board: () => (inTauri ? invoke<BoardView>("get_board") : devBoard()),
  refresh: () => invoke<void>("refresh"),
  moveCard: (cardId: string, columnId: string) => invoke<void>("move_card", { cardId, columnId }),
  addTask: (task: NewTask) => invoke<Card>("add_task", { task }),
  patchCard: (cardId: string, patch: CardPatch) => invoke<Card>("patch_card", { cardId, patch }),
  deleteCard: (cardId: string) => invoke<void>("delete_card", { cardId }),
  columns: () => invoke<Column[]>("get_columns"),
  setColumns: (columns: Column[]) => invoke<void>("set_columns", { columns }),
  resetColumns: () => invoke<void>("reset_columns"),
  settings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),
  jumpIn: (cardId: string) => invoke<string>("jump_in", { cardId }),
  startCard: (cardId: string, prompt?: string) => invoke<string>("start_card", { cardId, prompt: prompt ?? null }),
  stopCard: (cardId: string) => invoke<void>("stop_card", { cardId }),
  stopAll: () => invoke<number>("stop_all"),
  quotaHistory: (limit: number) => invoke<QuotaSample[]>("quota_history", { limit }),
  projects: () => invoke<[string, string][]>("list_projects"),
  statuslineWrapper: (originalCommand: string) => invoke<string>("statusline_wrapper", { originalCommand }),
  paths: () => invoke<Record<string, string>>("kari_paths"),
  installHooks: () => invoke<string>("install_hooks"),
  uninstallHooks: () => invoke<void>("uninstall_hooks"),
  summarizeCard: (cardId: string) => invoke<Summary>("summarize_card", { cardId }),
  calibration: () => invoke<Calibration>("get_calibration"),
  fetchUsageNow: () => invoke<QuotaSample>("fetch_usage_now"),
  proposal: () => invoke<Proposal | null>("get_proposal"),
  proposeNow: () => invoke<Proposal>("propose_now"),
  acceptProposal: (proposalId: string, cardIds?: string[]) => invoke<number>("accept_proposal", { proposalId, cardIds: cardIds ?? null }),
  snoozeProposal: (proposalId: string, minutes: number) => invoke<void>("snooze_proposal", { proposalId, minutes }),
  dismissProposal: (proposalId: string) => invoke<void>("dismiss_proposal", { proposalId }),
  stopProposal: (proposalId: string) => invoke<number>("stop_proposal", { proposalId }),
  proposalHistory: (limit: number) => invoke<Proposal[]>("proposal_history", { limit }),
  jobLog: (cardId: string, limit = 40) => invoke<JobLogEntry[]>("job_log", { cardId, limit }),
};

export function onBoardChanged(cb: () => void) {
  let t: number | undefined;
  const p = listen("board_changed", () => {
    if (t) window.clearTimeout(t);
    t = window.setTimeout(cb, 150);
  });
  return () => {
    p.then((un) => un());
  };
}

export function onNotice(cb: (n: { title: string; body: string; card_id: string | null }) => void) {
  const p = listen<{ title: string; body: string; card_id: string | null }>("notice", (e) => cb(e.payload));
  return () => {
    p.then((un) => un());
  };
}
