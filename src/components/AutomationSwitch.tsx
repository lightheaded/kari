import type { AutomationMode, NodeStatus } from "../types";
import { AUTOMATION_MODES } from "../types";

interface Props {
  nodes: NodeStatus[];
  /** The node filter. Empty means the switch acts on every node. */
  filter: string;
  onChange: (nodeId: string, mode: AutomationMode) => void;
}

/** The mode every node agrees on, or null when they differ. */
function shared(nodes: NodeStatus[]): AutomationMode | null {
  const modes = new Set(nodes.map((n) => n.automation_mode || "ask"));
  return modes.size === 1 ? ([...modes][0] as AutomationMode) : null;
}

/** Off, Ask or Auto, in one control. With a node filter on it sets that node.
 *  With no filter it sets every node that answers. */
export function AutomationSwitch({ nodes, filter, onChange }: Props) {
  const live = nodes.filter((n) => n.enabled && n.online);
  const scope = filter ? live.filter((n) => n.id === filter) : live;
  const current = shared(scope);
  const many = live.length > 1;
  const where = filter ? scope[0]?.name ?? filter : many ? "every node" : "";

  return (
    <div className="autoswitch" role="radiogroup" aria-label="Automatic behaviour">
      <span className="swlabel" title={`Automatic behaviour${where ? ` on ${where}` : ""}`}>
        auto
      </span>
      {AUTOMATION_MODES.map((m) => (
        <button
          key={m.value}
          role="radio"
          aria-checked={current === m.value}
          className={`swopt ${current === m.value ? "on" : ""} ${m.value}`}
          disabled={scope.length === 0}
          title={`${m.help}${where ? ` Applies to ${where}.` : ""}`}
          onClick={() => onChange(filter, m.value)}
        >
          {m.label}
        </button>
      ))}
      {current === null && (
        <span className="swmixed" title="The nodes do not agree. Pick one to set them all.">
          mixed
        </span>
      )}
    </div>
  );
}
