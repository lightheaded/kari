import type { Calibration, QuotaSample, QuotaWindow } from "../types";
import { fmtPct, relTime, resetIn, resetTitle } from "../util";

/** One window. The meter keeps its box when kari has no reading for the
 *  window, so the bars of two accounts stand in one column. Every window names
 *  its reset, or says why it has none. */
function Meter({ label, w }: { label: string; w: QuotaWindow | null }) {
  const pct = w ? Math.max(0, Math.min(100, w.used_percentage)) : 0;
  const cls = pct >= 90 ? "hot" : pct >= 70 ? "warn" : "";
  return (
    <div className="meter" title={resetTitle(label, w)}>
      <div className="l">
        <span>{label}</span>
        <b>
          {w ? `${pct.toFixed(0)}%` : "—"}
          <span className={w?.resets_at ? "mr" : "mr soft"}>{resetIn(w)}</span>
        </b>
      </div>
      <div className="bar">{w && <i className={cls} style={{ width: `${pct}%` }} />}</div>
    </div>
  );
}

interface Props {
  quota: QuotaSample | null;
  calibration?: Calibration | null;
  /** Node name. Shown before the meters when the board has more than one node. */
  label?: string;
  onHelp: () => void;
  onFill?: () => void;
}

export function QuotaBar({ quota, calibration, label, onHelp, onFill }: Props) {
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
  return (
    <div className="quota" title={`sampled ${relTime(quota.at)} ago via ${quota.source}${cal ? `\n${cal}` : ""}`}>
      {label && <span className="qlabel">{label}</span>}
      <Meter label="5-hour" w={quota.five_hour} />
      <Meter label="7-day" w={quota.seven_day} />
      {stale && (
        <span className="stale" title={`the newest sample is ${relTime(quota.at)} old`}>
          stale
        </span>
      )}
      {onFill && (
        <button className="btn sm ghost" onClick={onFill} title="Plan a run that fills the free quota">
          Fill the quota
        </button>
      )}
    </div>
  );
}
