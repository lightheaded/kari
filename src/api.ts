import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  BoardView,
  Calibration,
  Card,
  CardPatch,
  Column,
  HubBoard,
  JobLogEntry,
  NewNode,
  NewTask,
  NodePatch,
  NodeStatus,
  Proposal,
  QuotaSample,
  Settings,
  Summary,
} from "./types";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
/** A browser preview served by the Vite dev server, where fixtures stand in
 *  for the app core. A packaged app is served from its own protocol, so a
 *  missing bridge there is a fault to report, not a reason to read fixtures. */
const inDevServer = typeof window !== "undefined" && /^https?:\/\/localhost:\d+$/.test(window.location.origin);

/** The page has no way to reach the app core. Fail loudly: a silent fall back
 *  to fixtures inside a packaged app looks like a call that never answers. */
function noBridge<T>(what: string): Promise<T> {
  return Promise.reject(new Error(`${what}: this page cannot reach the app core (the Tauri bridge is missing)`));
}

/** Browser preview without Tauri: the Vite dev server serves `fixtures/*.json` at `/dev/*.json`.
 *  `board.json` comes from `cargo run -p kari-core --example board -- --json`. `settings.json` and
 *  `job-log.json` are optional. `docs/demo/` holds a dummy set for screenshots (`bun run screenshots`). */
async function devFixture<T>(name: string, fallback?: T): Promise<T> {
  const r = await fetch(`/dev/${name}.json`);
  if (!r.ok) {
    if (fallback !== undefined) return fallback;
    throw new Error(`no fixtures/${name}.json; run the board example with --json`);
  }
  return r.json();
}

/** The one node a single-machine board has. A fixture from an older kari has no nodes. */
function localNode(): NodeStatus {
  return {
    id: "local",
    name: "local",
    kind: "local",
    online: true,
    enabled: true,
    paired: true,
    ssh_host: null,
    address: null,
    remote_port: 0,
    version: null,
    api_version: null,
    remote_node_id: null,
    last_seen: null,
    error: null,
    lease: null,
    primary: true,
    away_mode: false,
    addresses: [],
  };
}

/** Read a board fixture. It holds either an old one-machine board or a hub board. */
function toHubBoard(json: BoardView | HubBoard): HubBoard {
  if ("nodes" in json) return json;
  const node = localNode();
  const tag = { node_id: node.id, node_name: node.name };
  return {
    columns: json.columns,
    hub_id: node.id,
    hub_name: node.name,
    primary: true,
    nodes: [node],
    cards: json.cards.map((c) => ({ ...c, ...tag })),
    quotas: [{ ...tag, quota: json.quota, calibration: json.calibration }],
    proposals: json.proposal ? [{ ...tag, proposal: json.proposal }] : [],
    generated_at: json.generated_at,
    scanning: json.scanning,
    herdr_connected: json.herdr_connected,
    hooks_installed: json.hooks_installed,
    hooks_port: json.hooks_port,
  };
}

export const api = {
  board: () =>
    inTauri
      ? invoke<HubBoard>("get_board")
      : inDevServer
        ? devFixture<BoardView | HubBoard>("board").then(toHubBoard)
        : noBridge<HubBoard>("board"),
  refresh: () => invoke<void>("refresh"),
  moveCard: (nodeId: string, cardId: string, columnId: string) => invoke<void>("move_card", { nodeId, cardId, columnId }),
  addTask: (nodeId: string, task: NewTask) => invoke<Card>("add_task", { nodeId, task }),
  patchCard: (nodeId: string, cardId: string, patch: CardPatch) => invoke<Card>("patch_card", { nodeId, cardId, patch }),
  deleteCard: (nodeId: string, cardId: string) => invoke<void>("delete_card", { nodeId, cardId }),
  columns: () => invoke<Column[]>("get_columns"),
  setColumns: (columns: Column[]) => invoke<void>("set_columns", { columns }),
  resetColumns: () => invoke<void>("reset_columns"),
  settings: () =>
    inTauri ? invoke<Settings>("get_settings") : inDevServer ? devFixture<Settings>("settings") : noBridge<Settings>("settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),
  jumpIn: (nodeId: string, cardId: string) => invoke<string>("jump_in", { nodeId, cardId }),
  startCard: (nodeId: string, cardId: string, prompt?: string) => invoke<string>("start_card", { nodeId, cardId, prompt: prompt ?? null }),
  stopCard: (nodeId: string, cardId: string) => invoke<void>("stop_card", { nodeId, cardId }),
  stopAll: () => invoke<number>("stop_all"),
  quotaHistory: (nodeId: string, limit: number) => invoke<QuotaSample[]>("quota_history", { nodeId, limit }),
  projects: (nodeId: string) => invoke<[string, string][]>("list_projects", { nodeId }),
  statuslineWrapper: (originalCommand: string) => invoke<string>("statusline_wrapper", { originalCommand }),
  paths: () => invoke<Record<string, string>>("kari_paths"),
  installHooks: () => invoke<string>("install_hooks"),
  uninstallHooks: () => invoke<void>("uninstall_hooks"),
  summarizeCard: (nodeId: string, cardId: string) => invoke<Summary>("summarize_card", { nodeId, cardId }),
  calibration: () => invoke<Calibration>("get_calibration"),
  fetchUsageNow: () => invoke<QuotaSample>("fetch_usage_now"),
  proposal: (nodeId: string) => invoke<Proposal | null>("get_proposal", { nodeId }),
  proposeNow: (nodeId: string) => invoke<Proposal>("propose_now", { nodeId }),
  acceptProposal: (nodeId: string, proposalId: string, cardIds?: string[]) =>
    invoke<number>("accept_proposal", { nodeId, proposalId, cardIds: cardIds ?? null }),
  snoozeProposal: (nodeId: string, proposalId: string, minutes: number) => invoke<void>("snooze_proposal", { nodeId, proposalId, minutes }),
  dismissProposal: (nodeId: string, proposalId: string) => invoke<void>("dismiss_proposal", { nodeId, proposalId }),
  stopProposal: (nodeId: string, proposalId: string) => invoke<number>("stop_proposal", { nodeId, proposalId }),
  proposalHistory: (nodeId: string, limit: number) => invoke<Proposal[]>("proposal_history", { nodeId, limit }),
  listNodes: () => invoke<NodeStatus[]>("list_nodes"),
  addNode: (node: NewNode) => invoke<NodeStatus>("add_node", { node }),
  updateNode: (nodeId: string, patch: NodePatch) => invoke<NodeStatus>("update_node", { nodeId, patch }),
  removeNode: (nodeId: string) => invoke<void>("remove_node", { nodeId }),
  pairNode: (nodeId: string) => invoke<string>("pair_node", { nodeId }),
  claimPrimary: () => invoke<string>("claim_primary"),
  answerPermission: (nodeId: string, permissionId: string, behavior: "allow" | "deny") =>
    invoke<void>("answer_permission", { nodeId, permissionId, behavior }),
  setAwayMode: (nodeId: string, on: boolean) => invoke<void>("set_away_mode", { nodeId, on }),
  pairingCode: () => invoke<string>("pairing_code"),
  jobLog: (nodeId: string, cardId: string, limit = 40) =>
    inTauri
      ? invoke<JobLogEntry[]>("job_log", { nodeId, cardId, limit })
      : devFixture<Record<string, JobLogEntry[]>>("job-log", {}).then((m) => (m[cardId] ?? []).slice(0, limit)),
};

/** The board changed on one node. The callback reloads the whole board. */
export function onBoardChanged(cb: () => void) {
  if (!inTauri) return () => {};
  let t: number | undefined;
  const p = listen<{ node_id: string }>("board_changed", () => {
    if (t) window.clearTimeout(t);
    t = window.setTimeout(cb, 150);
  });
  return () => {
    p.then((un) => un());
  };
}

export interface Notice {
  title: string;
  body: string;
  card_id: string | null;
  node_id: string;
  node_name: string;
}

export function onNotice(cb: (n: Notice) => void) {
  if (!inTauri) return () => {};
  const p = listen<Notice>("notice", (e) => cb(e.payload));
  return () => {
    p.then((un) => un());
  };
}
