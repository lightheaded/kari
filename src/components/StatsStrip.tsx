import type { Calibration, NodeQuota, NodeStatus, QuotaSample, QuotaWindow } from "../types";
import { fmtPct, nodeDot, relTime, untilTime } from "../util";

function Meter({ label, w }: { label: string; w: QuotaWindow | null }) {
  if (!w) return <span className="mnone">{label} —</span>;
  const pct = Math.max(0, Math.min(100, w.used_percentage));
  const cls = pct >= 90 ? "hot" : pct >= 70 ? "warn" : "";
  return (
    <span className="m" title={w.resets_at ? `${label}: resets in ${untilTime(w.resets_at)}` : label}>
      <span className="mk">{label}</span>
      <span className="bar">
        <i className={cls} style={{ width: `${pct}%` }} />
      </span>
      <b>{pct.toFixed(0)}%</b>
      {w.resets_at && <span className="mr">{untilTime(w.resets_at)}</span>}
    </span>
  );
}

interface RowProps {
  name: string;
  quota: QuotaSample | null;
  calibration: Calibration | null;
  node: NodeStatus | undefined;
  /** Hide the node name and the dot on a board with one node. */
  showName: boolean;
  /** The node filter names this node. */
  selected: boolean;
  onFilter: () => void;
  onFill: () => void;
  onHelp: () => void;
  /** A click on "stale" asks the usage endpoint now. Only the local node can. */
  onRefresh?: () => void;
  refreshing?: boolean;
}

function Row({ name, quota, calibration, node, showName, selected, onFilter, onFill, onHelp, onRefresh, refreshing }: RowProps) {
  const stale = quota ? (Date.now() - new Date(quota.at).getTime()) / 1000 > 300 : false;
  const cal = calibration
    ? `\ncalibration ${fmtPct(calibration.pct_per_mtok)} of the 5-hour window per 1M weighted tokens (${calibration.source})`
    : "";
  return (
    <div className={`srow ${selected ? "sel" : ""}`}>
      {showName && (
        <button
          className="sname"
          onClick={onFilter}
          title={selected ? "Show every node again" : `Show only the cards of ${name}`}
        >
          {node && <span className={nodeDot(node)} />}
          {name}
        </button>
      )}
      {quota ? (
        <span className="smeters" title={`sampled ${relTime(quota.at)} ago via ${quota.source}${cal}`}>
          <Meter label="5h" w={quota.five_hour} />
          <Meter label="7d" w={quota.seven_day} />
        </span>
      ) : (
        <button className="snone" onClick={onHelp} title="How to switch quota tracking on">
          no quota sample. Install the status line wrapper in Settings.
        </button>
      )}
      {stale && (
        <button
          className="stale"
          disabled={!onRefresh || refreshing}
          onClick={onRefresh}
          title={[
            `The newest sample is ${relTime(quota!.at)} old.`,
            "kari reads the quota from the status line of a running Claude Code session. With no session, the numbers age.",
            onRefresh ? "Click to ask the usage endpoint now." : "Only this machine can ask the usage endpoint.",
          ].join("\n")}
        >
          {refreshing ? "asking…" : "stale"}
        </button>
      )}
      {quota && (
        <button className="btn ghost sm sfill" onClick={onFill} title="Plan a run that fills the free quota">
          Fill
        </button>
      )}
    </div>
  );
}

interface Props {
  quotas: NodeQuota[];
  nodes: NodeStatus[];
  /** The node the board filters on, or an empty string for every node. */
  filter: string;
  onFilter: (nodeId: string) => void;
  onFill: (nodeId: string) => void;
  onHelp: () => void;
  /** Ask the usage endpoint now. Only this machine holds the login token. */
  onRefresh: () => void;
  refreshing: boolean;
}

/** One row per node under the top bar: both windows, both reset times, and a
 *  Fill button. Above four nodes the rows go two to a line, and the strip
 *  scrolls after that, so the top bar itself never grows. */
export function StatsStrip({ quotas, nodes, filter, onFilter, onFill, onHelp, onRefresh, refreshing }: Props) {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const many = nodes.length > 1;
  const rows =
    quotas.length > 0
      ? quotas
      : [{ node_id: nodes[0]?.id ?? "local", node_name: nodes[0]?.name ?? "", quota: null, calibration: null }];
  return (
    <div className={`stats ${rows.length > 4 ? "wide" : ""}`}>
      {rows.map((q) => (
        <Row
          key={q.node_id}
          name={q.node_name}
          quota={q.quota}
          calibration={"calibration" in q ? (q.calibration as Calibration) : null}
          node={byId.get(q.node_id)}
          showName={many}
          selected={filter === q.node_id}
          onFilter={() => onFilter(filter === q.node_id ? "" : q.node_id)}
          onFill={() => onFill(q.node_id)}
          onHelp={onHelp}
          onRefresh={byId.get(q.node_id)?.kind === "local" ? onRefresh : undefined}
          refreshing={refreshing}
        />
      ))}
    </div>
  );
}
