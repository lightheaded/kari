import { useMemo, useState } from "react";
import { api } from "../api";
import type { Proposal } from "../types";
import { fmtM, fmtPct, relTime, untilTime } from "../util";

const TRIGGER_LABEL: Record<string, string> = {
  weekly_reset: "Weekly window resets soon",
  idle_five_hour: "The 5-hour window is free",
  manual: "You asked for a plan",
};

interface Props {
  proposal: Proposal;
  onClose: () => void;
  onAction: (fn: () => Promise<unknown>, ok?: string) => Promise<void>;
  onSelectCard: (cardId: string) => void;
}

export function ProposalPanel({ proposal: p, onClose, onAction, onSelectCard }: Props) {
  const runnable = p.items.filter((i) => !i.job_id);
  const [picked, setPicked] = useState<string[]>(() => runnable.map((i) => i.card_id));
  const accepted = p.state === "accepted";
  const started = p.items.filter((i) => i.job_id);
  const total = useMemo(
    () => p.items.filter((i) => picked.includes(i.card_id)).reduce((n, i) => n + i.estimate.pct_five_hour, 0),
    [p.items, picked],
  );
  const toggle = (id: string) => setPicked((v) => (v.includes(id) ? v.filter((x) => x !== id) : [...v, id]));

  return (
    <div className="proposal">
      <header>
        <h3>
          {accepted ? (p.auto ? "Autopilot started these" : "Started these") : TRIGGER_LABEL[p.trigger] ?? "Plan"}
        </h3>
        <div className="spacer" />
        <button className="btn ghost sm" onClick={onClose} aria-label="Close">
          ✕
        </button>
      </header>
      <div className="why">{p.reason}.</div>
      <div className="numbers">
        <span>
          budget <b>{fmtPct(p.budget_pct)}</b>
        </span>
        <span>
          plan <b>{fmtPct(accepted ? p.total_pct : total)}</b>
        </span>
        <span>
          window after <b>{fmtPct(p.used_pct_before + (accepted ? p.total_pct : total))}</b>
        </span>
        {!accepted && <span>expires in {untilTime(p.expires_at)}</span>}
        {accepted && p.accepted_at && <span>started {relTime(p.accepted_at)} ago</span>}
      </div>
      <ul className="items">
        {p.items.map((i) => (
          <li key={i.card_id}>
            {!accepted && (
              <input type="checkbox" checked={picked.includes(i.card_id)} onChange={() => toggle(i.card_id)} aria-label={`Include ${i.title}`} />
            )}
            <div className="t">
              <button className="linkish" onClick={() => onSelectCard(i.card_id)} title="Open this card">
                {i.title}
              </button>
              <div className="hint">
                {i.project_name ?? "no project"} · {fmtPct(i.estimate.pct_five_hour)} · {fmtM(i.estimate.weighted_tokens)} weighted · {i.estimate.source}
                {i.model ? ` · ${i.model}` : ""}
              </div>
              {i.error && <div className="err">{i.error}</div>}
              {i.job_id && <div className="hint">job {i.job_id}</div>}
            </div>
          </li>
        ))}
      </ul>
      {p.skipped > 0 && <div className="hint">{p.skipped} more card(s) did not fit the budget.</div>}
      <footer>
        {accepted ? (
          <>
            <button
              className="btn danger sm"
              disabled={started.length === 0}
              onClick={() => onAction(() => api.stopProposal(p.id), "Stopped the started jobs")}
            >
              ■ Stop these jobs
            </button>
            <div className="spacer" />
            <button className="btn sm" onClick={onClose}>
              Close
            </button>
          </>
        ) : (
          <>
            <button className="btn primary sm" disabled={picked.length === 0} onClick={() => onAction(() => api.acceptProposal(p.id, picked), "Started")}>
              ▶ Start {picked.length === p.items.length ? "all" : `${picked.length}`}
            </button>
            <button className="btn sm" onClick={() => onAction(() => api.snoozeProposal(p.id, 60), "Snoozed for an hour")}>
              Snooze 1 hour
            </button>
            <div className="spacer" />
            <button className="btn ghost sm" onClick={() => onAction(() => api.dismissProposal(p.id), "Dismissed")}>
              Dismiss
            </button>
          </>
        )}
      </footer>
    </div>
  );
}
