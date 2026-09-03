import { useMemo, useState } from "react";
import { api } from "../api";
import type { Proposal, ProposalItem } from "../types";
import { fmtM, fmtPct, relTime, untilTime } from "../util";

const TRIGGER_LABEL: Record<string, string> = {
  weekly_reset: "Weekly window resets soon",
  idle_five_hour: "The 5-hour window is free",
  manual: "You asked for a plan",
};

const SKIP_LABEL: Record<string, string> = {
  budget: "over budget",
  slots: "parallel cap",
};

interface Segment {
  key: string;
  left: number;
  width: number;
  tone: "plan" | "plan-alt" | "over";
  title: string;
}

const pos = (n: number) => `${Math.max(0, Math.min(100, n))}%`;
const span = (left: number, width: number) => `${Math.max(0, Math.min(100, left + width) - Math.max(0, left))}%`;

// The 5-hour window as one bar: what is used, what the picked tasks add, and where the budget ends.
function BudgetBar({ usedBefore, budget, picked }: { usedBefore: number; budget: number; picked: ProposalItem[] }) {
  const line = usedBefore + budget;
  const segs: Segment[] = [];
  let cursor = usedBefore;
  picked.forEach((i, idx) => {
    const pct = i.estimate.pct_five_hour;
    const start = cursor;
    const end = start + pct;
    cursor = end;
    const inside = Math.max(0, Math.min(end, line) - start);
    const over = Math.max(0, end - Math.max(start, line));
    if (inside > 0) {
      segs.push({ key: `${i.card_id}:in`, left: start, width: inside, tone: idx % 2 ? "plan-alt" : "plan", title: `${i.title} · ${fmtPct(pct)}` });
    }
    if (over > 0) {
      segs.push({ key: `${i.card_id}:over`, left: Math.max(start, line), width: over, tone: "over", title: `${i.title} · ${fmtPct(pct)} · over the budget` });
    }
  });
  const total = cursor - usedBefore;
  const after = cursor;
  const left = budget - total;
  const state = after > 100 ? "spill" : left < 0 ? "over" : "";
  return (
    <div className={`budget ${state}`}>
      <div className="track" role="img" aria-label={`5-hour window: ${fmtPct(usedBefore)} used, plan adds ${fmtPct(total)}, budget ${fmtPct(budget)}`}>
        <i className="used" style={{ width: pos(usedBefore) }} title={`used now ${fmtPct(usedBefore)}`} />
        {segs.map((s) => (
          <i key={s.key} className={s.tone} style={{ left: pos(s.left), width: span(s.left, s.width) }} title={s.title} />
        ))}
        <b className="line" style={{ left: pos(line) }} title={`budget ends at ${fmtPct(line)} of the window`} />
      </div>
      <div className="legend">
        <span>
          used <b>{fmtPct(usedBefore)}</b>
        </span>
        <span>
          plan <b>{fmtPct(total)}</b> of <b>{fmtPct(budget)}</b>
        </span>
        <span>
          after <b>{fmtPct(after)}</b>
        </span>
        <span className="spacer" />
        {after > 100 ? (
          <span className="verdict spill">past the end of the window</span>
        ) : left < 0 ? (
          <span className="verdict over">over the budget by {fmtPct(-left)}</span>
        ) : (
          <span className="verdict ok">{fmtPct(left)} of the budget left</span>
        )}
      </div>
    </div>
  );
}

interface Props {
  proposal: Proposal;
  onClose: () => void;
  onAction: (fn: () => Promise<unknown>, ok?: string) => Promise<void>;
  onSelectCard: (cardId: string) => void;
}

export function ProposalPanel({ proposal: p, onClose, onAction, onSelectCard }: Props) {
  const accepted = p.state === "accepted";
  const runnable = p.items.filter((i) => !i.job_id);
  const [picked, setPicked] = useState<string[]>(() => runnable.filter((i) => i.fits).map((i) => i.card_id));
  const started = p.items.filter((i) => i.job_id);
  const pickedItems = useMemo(() => p.items.filter((i) => picked.includes(i.card_id)), [p.items, picked]);
  const total = pickedItems.reduce((n, i) => n + i.estimate.pct_five_hour, 0);
  const overBudget = !accepted && total > p.budget_pct + 1e-9;
  const toggle = (id: string) => setPicked((v) => (v.includes(id) ? v.filter((x) => x !== id) : [...v, id]));

  const fitting = p.items.filter((i) => i.fits);
  const unfit = p.items.filter((i) => !i.fits);
  // After the start, the panel only lists what ran or failed to run.
  const shown = accepted ? p.items.filter((i) => i.job_id || i.error) : fitting;

  const row = (i: ProposalItem) => (
    <li key={i.card_id} className={picked.includes(i.card_id) || accepted ? "on" : "off"}>
      {!accepted && (
        <input type="checkbox" checked={picked.includes(i.card_id)} onChange={() => toggle(i.card_id)} aria-label={`Include ${i.title}`} />
      )}
      <div className="t">
        <div className="row">
          <button className="linkish" onClick={() => onSelectCard(i.card_id)} title="Open this card">
            {i.title}
          </button>
          {!i.fits && !accepted && <span className="skip">{SKIP_LABEL[i.skip_reason ?? ""] ?? "did not fit"}</span>}
          <span className="pct">{fmtPct(i.estimate.pct_five_hour)}</span>
        </div>
        <div className="hint">
          {i.project_name ?? "no project"} · {fmtM(i.estimate.weighted_tokens)} weighted · {i.estimate.source}
          {i.model ? ` · ${i.model}` : ""}
        </div>
        {i.error && <div className="err">{i.error}</div>}
        {i.job_id && <div className="hint">job {i.job_id}</div>}
      </div>
    </li>
  );

  return (
    <div className="proposal">
      <header>
        <h3>{accepted ? (p.auto ? "Autopilot started these" : "Started these") : TRIGGER_LABEL[p.trigger] ?? "Plan"}</h3>
        <div className="spacer" />
        <span className="when">
          {!accepted && `expires in ${untilTime(p.expires_at)}`}
          {accepted && p.accepted_at && `started ${relTime(p.accepted_at)} ago`}
        </span>
        <button className="btn ghost sm" onClick={onClose} aria-label="Close">
          ✕
        </button>
      </header>
      <div className="why">{p.reason}.</div>
      <BudgetBar usedBefore={p.used_pct_before} budget={p.budget_pct} picked={accepted ? started : pickedItems} />
      <ul className="items">
        {shown.map(row)}
        {!accepted && unfit.length > 0 && (
          <>
            <li className="group">
              <span>Did not fit the budget</span>
              <span className="hint">Pick a card to start it anyway.</span>
            </li>
            {unfit.map(row)}
          </>
        )}
      </ul>
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
            <button
              className={`btn primary sm${overBudget ? " over" : ""}`}
              disabled={picked.length === 0}
              title={overBudget ? "The picked tasks need more than the budget. Start them anyway." : undefined}
              onClick={() => onAction(() => api.acceptProposal(p.id, picked), "Started")}
            >
              ▶ Start {picked.length === runnable.length ? "all" : `${picked.length}`}
              {overBudget ? " · over budget" : ""}
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
