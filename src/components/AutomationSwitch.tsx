import type { AccountQuota, AutomationMode, NodeStatus } from "../types";
import { AUTOMATION_MODES } from "../types";
import { accountScope, sharedMode } from "../util";

interface SwitchProps {
  /** The machines the write lands on. An empty scope disables the control. */
  scope: NodeStatus[];
  /** What the write covers, named in every tooltip: a machine, an account, or
   *  every node. Empty when there is only one thing it could mean. */
  where: string;
  /** The label in front of the buttons. None in a place too tight for one. */
  label?: string;
  /** What the control is for, for a reader who cannot see the label. */
  ariaLabel: string;
  /** The narrow variant, for a quota row. */
  compact?: boolean;
  onPick: (mode: AutomationMode) => void;
}

/** Off, Ask or Auto, in one control, over whatever set of machines it is given.
 *
 *  The mode itself lives in each machine's settings, because the planner that
 *  reads it runs there. A switch is a scope of one write: this machine, this
 *  account, or every node. `mixed` says the machines in the scope do not agree,
 *  which a single write puts right. */
function Switch({ scope, where, label, ariaLabel, compact, onPick }: SwitchProps) {
  const current = sharedMode(scope);
  return (
    <div className={`autoswitch${compact ? " sm" : ""}`} role="radiogroup" aria-label={ariaLabel}>
      {label && (
        <span className="swlabel" title={`Automatic behaviour${where ? ` on ${where}` : ""}`}>
          {label}
        </span>
      )}
      {AUTOMATION_MODES.map((m) => (
        <button
          key={m.value}
          role="radio"
          aria-checked={current === m.value}
          className={`swopt ${current === m.value ? "on" : ""} ${m.value}`}
          disabled={scope.length === 0}
          title={`${m.help}${where ? ` Applies to ${where}.` : ""}`}
          onClick={() => onPick(m.value)}
        >
          {m.label}
        </button>
      ))}
      {current === null && (
        <span className="swmixed" title="The machines do not agree. Pick one to set them all.">
          mixed
        </span>
      )}
    </div>
  );
}

interface Props {
  nodes: NodeStatus[];
  /** The node filter. Empty means the switch acts on every node. */
  filter: string;
  onChange: (nodeId: string, mode: AutomationMode) => void;
}

/** The switch in the top bar. With a node filter on it sets that node. With no
 *  filter it sets every node that answers, whatever account each spends. */
export function AutomationSwitch({ nodes, filter, onChange }: Props) {
  const live = nodes.filter((n) => n.enabled && n.online);
  const scope = filter ? live.filter((n) => n.id === filter) : live;
  const many = live.length > 1;
  return (
    <Switch
      scope={scope}
      where={filter ? scope[0]?.name ?? filter : many ? "every node" : ""}
      label="auto"
      ariaLabel="Automatic behaviour"
      onPick={(m) => onChange(filter, m)}
    />
  );
}

interface AccountProps {
  row: AccountQuota;
  /** Every node on the board. The switch takes the ones on this row's account. */
  nodes: NodeStatus[];
  onChange: (mode: AutomationMode) => void;
}

/** The switch beside one quota meter: the machines signed in to that account,
 *  and no others.
 *
 *  Quota belongs to the account, so this is the control that answers "leave
 *  that subscription for its own work and spend this one". The switch in the
 *  top bar cannot say it: with no filter it covers every machine, and with one
 *  it covers a single machine, while a subscription is usually neither. */
export function AccountAutomationSwitch({ row, nodes, onChange }: AccountProps) {
  const scope = accountScope(nodes, row);
  return (
    <Switch
      scope={scope}
      where={scope.length > 1 ? `the ${scope.length} machines on ${row.label}` : scope[0]?.name ?? row.label}
      ariaLabel={`Automatic behaviour on ${row.label}`}
      compact
      onPick={onChange}
    />
  );
}
