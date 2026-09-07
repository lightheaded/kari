import { useEffect, useRef, useState } from "react";
import type { AccountQuota, NodeStatus, QuotaWindow } from "../types";
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

/** The account's name, and a click to change it. The name is this device's own
 *  label for the account; it goes nowhere near the Claude account itself. */
function Name({ row, onRename }: { row: AccountQuota; onRename: (alias: string) => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(row.alias ?? "");
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing) input.current?.select();
  }, [editing]);

  function commit() {
    setEditing(false);
    if (draft.trim() !== (row.alias ?? "")) onRename(draft.trim());
  }

  if (editing) {
    return (
      <input
        ref={input}
        className="aname edit"
        value={draft}
        placeholder={row.account?.display_name ?? row.account?.email ?? row.label}
        aria-label="Name for this account"
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
          if (e.key === "Escape") {
            setDraft(row.alias ?? "");
            setEditing(false);
          }
        }}
      />
    );
  }
  const known = [row.account?.display_name, row.account?.email].filter(Boolean).join(" · ");
  return (
    <button
      className="aname"
      onClick={() => {
        setDraft(row.alias ?? "");
        setEditing(true);
      }}
      title={[known || "kari could not read this node's Claude Code account", "Click to name this account."]
        .filter(Boolean)
        .join("\n")}
    >
      {row.label}
    </button>
  );
}

interface RowProps {
  row: AccountQuota;
  byId: Map<string, NodeStatus>;
  /** Name the machines on the row. Off on a board with a single node. */
  showNodes: boolean;
  onFill: () => void;
  onHelp: () => void;
  onRename: (alias: string) => void;
  /** Ask the usage endpoint now. Only this machine holds the login token. */
  onRefresh?: () => void;
  refreshing?: boolean;
}

function Row({ row, byId, showNodes, onFill, onHelp, onRename, onRefresh, refreshing }: RowProps) {
  const quota = row.quota;
  const stale = quota ? (Date.now() - new Date(quota.at).getTime()) / 1000 > 300 : false;
  const cal = row.calibration
    ? `\ncalibration ${fmtPct(row.calibration.pct_per_mtok)} of the 5-hour window per 1M weighted tokens (${row.calibration.source})`
    : "";
  return (
    <div className="srow">
      <Name row={row} onRename={onRename} />
      {showNodes && (
        <span className="anodes" title={`${row.node_names.length} machine(s) spend this quota`}>
          {row.node_ids.map((id, i) => {
            const n = byId.get(id);
            return (
              <span key={id} className="anode">
                <span className={n ? nodeDot(n) : "dot"} />
                {row.node_names[i]}
              </span>
            );
          })}
        </span>
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
        <button
          className="btn ghost sm sfill"
          onClick={onFill}
          title={
            row.node_ids.length > 1
              ? `Plan a run that fills the free quota, on ${row.node_names[0]}`
              : "Plan a run that fills the free quota"
          }
        >
          Fill
        </button>
      )}
    </div>
  );
}

interface Props {
  accounts: AccountQuota[];
  nodes: NodeStatus[];
  onFill: (nodeId: string) => void;
  onHelp: () => void;
  onRename: (key: string, alias: string) => void;
  /** Ask the usage endpoint now. Only this machine holds the login token. */
  onRefresh: () => void;
  refreshing: boolean;
}

/** One row per Claude Code account under the top bar: both windows, both reset
 *  times, the machines that spend it, and a Fill button.
 *
 *  A row per account rather than per node, because that is what the quota
 *  belongs to. Two machines signed in to one login draw down a single 5-hour
 *  window; a meter each would read as two budgets and send the planner after
 *  quota that is already spent. Filtering the board stays with the node chips
 *  below, which can name one machine — a row here covers several.
 *
 *  Above four rows they go two to a line, and the strip scrolls after that, so
 *  the top bar itself never grows. */
export function StatsStrip({ accounts, nodes, onFill, onHelp, onRename, onRefresh, refreshing }: Props) {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const rows: AccountQuota[] =
    accounts.length > 0
      ? accounts
      : [
          {
            key: `node:${nodes[0]?.id ?? "local"}`,
            label: nodes[0]?.name ?? "",
            alias: null,
            account: null,
            node_ids: [nodes[0]?.id ?? "local"],
            node_names: [nodes[0]?.name ?? ""],
            quota: null,
            calibration: null,
          },
        ];
  return (
    <div className={`stats ${rows.length > 4 ? "wide" : ""}`}>
      {rows.map((row) => (
        <Row
          key={row.key}
          row={row}
          byId={byId}
          showNodes={nodes.length > 1}
          onFill={() => onFill(row.node_ids[0])}
          onHelp={onHelp}
          onRename={(alias) => onRename(row.key, alias)}
          // The login token lives on this machine only, so the endpoint is
          // reachable only for an account this machine is itself signed in to.
          onRefresh={row.node_ids.some((id) => byId.get(id)?.kind === "local") ? onRefresh : undefined}
          refreshing={refreshing}
        />
      ))}
    </div>
  );
}
