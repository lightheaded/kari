import { api } from "./api";
import type { Act } from "./toasts";
import type { AccountQuota, AutomationMode, NodeStatus } from "./types";
import { AUTOMATION_MODES } from "./types";
import { accountScope, sharedMode } from "./util";

const label = (m: AutomationMode) => AUTOMATION_MODES.find((x) => x.value === m)?.label ?? m;

/** Set what a node does by itself, and offer the way back.
 *
 *  An empty node id means every node that answers. The undo holds only when
 *  the nodes agreed on one mode before, because then there is one mode to go
 *  back to. The desktop top bar and the phone board both call this. */
export function setAutomation(nodes: NodeStatus[], nodeId: string, mode: AutomationMode, run: Act): void {
  const scope = nodeId ? nodes.filter((n) => n.id === nodeId) : nodes.filter((n) => n.enabled && n.online);
  const was = sharedMode(scope);
  const back =
    was && was !== mode
      ? { done: `Automation back to ${label(was)}`, run: () => api.setAutomationMode(nodeId, was) }
      : undefined;
  void run(() => api.setAutomationMode(nodeId, mode), `Automation: ${label(mode)}`, back);
}

/** Set what the machines on one account do by themselves.
 *
 *  Quota belongs to the account, so this is how one subscription is kept for
 *  its own work while another is spent: Off on the account to reserve, Auto on
 *  the account to burn. The write reaches every machine signed in to that
 *  login, because they draw down one window between them.
 *
 *  The undo holds only when those machines agreed on one mode before, as it
 *  does for a node: with no single mode to go back to there is nothing honest
 *  to put back. */
export function setAccountAutomation(nodes: NodeStatus[], row: AccountQuota, mode: AutomationMode, run: Act): void {
  const was = sharedMode(accountScope(nodes, row));
  const back =
    was && was !== mode
      ? {
          done: `${row.label} back to ${label(was)}`,
          run: () => api.setAccountAutomationMode(row.key, was),
        }
      : undefined;
  void run(() => api.setAccountAutomationMode(row.key, mode), `${row.label}: ${label(mode)}`, back);
}
