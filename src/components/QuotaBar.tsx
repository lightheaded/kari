import type { Calibration, QuotaSample, QuotaWindow } from "../types";
import { clock, fmtPct, relTime, untilTime } from "../util";

function Meter({ label, w, help }: { label: string; w: QuotaWindow | null; help: string }) {
  if (!w) return null;
  const pct = Math.max(0, Math.min(100, w.used_percentage));
  const cls = pct >= 90 ? "hot" : pct >= 70 ? "warn" : "";
  return (
    <div className="meter" title={`${help}${w.resets_at ? `\nResets ${clock(w.resets_at)}, in ${untilTime(w.resets_at)}.` : ""}`}>
      <div className="l">
        <span>{label}</span>
        <b>
          {pct.toFixed(0)}%{w.resets_at ? ` · resets in ${untilTime(w.resets_at)}` : ""}
        </b>
      </div>
      <div className="bar">
        <i className={cls} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

interface Props {
  quota: QuotaSample | null;
  calibration?: Calibration | null;
  onHelp: () => void;
  onFill?: () => void;
  /** The user clicked "stale": ask the usage endpoint now. */
  onRefresh?: () => void;
  refreshing?: boolean;
}

export function QuotaBar({ quota, calibration, onHelp, onFill, onRefresh, refreshing }: Props) {
  if (!quota) {
    return (
      <div className="quota">
        <button className="btn sm ghost none" onClick={onHelp} title="How to enable quota tracking">
          No quota sample yet. Install the status line wrapper in Settings.
        </button>
      </div>
    );
  }
  const ageSec = (Date.now() - new Date(quota.at).getTime()) / 1000;
  const stale = ageSec > 300;
  const cal = calibration
    ? `calibration ${fmtPct(calibration.pct_per_mtok)} of the 5-hour window per 1M weighted tokens (${calibration.source}${
        calibration.samples ? `, ${calibration.samples} pairs` : ""
      })`
    : "";
  const staleTip = [
    `The newest sample is ${relTime(quota.at)} old.`,
    "kari reads the quota from the status line of a running Claude Code session. With no session, the numbers age.",
    "Click to ask the usage endpoint now. Settings → Quota tracking can do this by itself.",
  ].join("\n");
  return (
    <div className="quota" title={`sampled ${relTime(quota.at)} ago via ${quota.source}${cal ? `\n${cal}` : ""}`}>
      <Meter label="5-hour" w={quota.five_hour} help="Share of the rolling 5-hour window that is used." />
      <Meter label="7-day" w={quota.seven_day} help="Share of the rolling 7-day window that is used." />
      {stale && (
        <button className="stale" title={staleTip} onClick={onRefresh} disabled={refreshing || !onRefresh}>
          {refreshing ? "asking…" : "stale ↻"}
        </button>
      )}
      {onFill && (
        <button className="btn sm ghost" onClick={onFill} title="Plan a run that fills the free quota">
          Fill the quota
        </button>
      )}
    </div>
  );
}
