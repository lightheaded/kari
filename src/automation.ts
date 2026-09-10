import { api } from "./api";
import type { Act } from "./toasts";
import type { AutomationMode, NodeStatus } from "./types";
import { AUTOMATION_MODES } from "./types";

const label = (m: AutomationMode) => AUTOMATION_MODES.find((x) => x.value === m)?.label ?? m;

/** Set what a node does by itself, and offer the way back.
 *
 *  An empty node id means every node that answers. The undo holds only when
 *  the nodes agreed on one mode before, because then there is one mode to go
 *  back to. The desktop top bar and the phone board both call this. */
export function setAutomation(nodes: NodeStatus[], nodeId: string, mode: AutomationMode, run: Act): void {
  const scope = nodeId ? nodes.filter((n) => n.id === nodeId) : nodes.filter((n) => n.enabled && n.online);
  const modes = new Set(scope.map((n) => n.automation_mode || "ask"));
  const was = modes.size === 1 ? ([...modes][0] as AutomationMode) : null;
  const back =
    was && was !== mode
      ? { done: `Automation back to ${label(was)}`, run: () => api.setAutomationMode(nodeId, was) }
      : undefined;
  void run(() => api.setAutomationMode(nodeId, mode), `Automation: ${label(mode)}`, back);
}
