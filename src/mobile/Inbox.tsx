import { useMemo, useState } from "react";
import { api } from "../api";
import type { DerivedState, HubBoard } from "../types";
import { QuotaBar } from "../components/QuotaBar";
import { ProposalPanel } from "../components/Proposals";
import { sortCards } from "../util";
import { MobileCard } from "./MobileCard";

/** The states that wait for a person, in the order they matter. */
const NEEDS_YOU: DerivedState[] = ["needs_approval", "needs_decision", "my_turn", "validate", "waiting_on_others"];

interface Props {
  board: HubBoard;
  onOpen: (node: string, id: string) => void;
  onAction: (fn: () => Promise<unknown>, ok?: string) => Promise<void>;
}

/** What waits for the user: the quota per node, the open plans, then every card that needs a person. */
export function Inbox({ board, onOpen, onAction }: Props) {
  const [planHidden, setPlanHidden] = useState<Set<string>>(() => new Set());
  const nodeById = useMemo(() => new Map(board.nodes.map((n) => [n.id, n])), [board.nodes]);
  const many = board.nodes.length > 1;
  const cards = useMemo(
    () => board.cards.filter((c) => NEEDS_YOU.includes(c.state) && !c.card.archived).sort(sortCards),
    [board.cards],
  );
  const working = board.cards.filter((c) => c.state === "working").length;

  return (
    <div className="minbox">
      <header className="mhead">
        <span className="wordmark">kari</span>
        <span className="mhead-meta">
          {working} working · {cards.length} need you
        </span>
        <button className="btn ghost sm" onClick={() => onAction(() => api.refresh(), "Refreshing")} title="Rescan now">
          ↻
        </button>
      </header>

      {/* One bar per account, not per node: nodes on one Claude Code login
          share a window, and a bar each would read as two budgets. */}
      <div className="mquotas">
        {(board.accounts ?? [])
          .filter((a) => a.quota)
          .map((a) => (
            <QuotaBar
              key={a.key}
              quota={a.quota}
              calibration={a.calibration}
              label={many ? a.label : undefined}
              onHelp={() => {}}
              onFill={() => onAction(() => api.proposeNow(a.node_ids[0]), "Plan ready")}
            />
          ))}
      </div>

      {board.proposals
        .filter((p) => !planHidden.has(`${p.node_id}:${p.proposal.id}`))
        .map((p) => (
          <ProposalPanel
            key={`${p.node_id}:${p.proposal.id}`}
            proposal={p.proposal}
            nodeId={p.node_id}
            nodeName={many ? p.node_name : undefined}
            onClose={() => setPlanHidden((h) => new Set(h).add(`${p.node_id}:${p.proposal.id}`))}
            onAction={onAction}
            onSelectCard={(id) => onOpen(p.node_id, id)}
          />
        ))}

      {cards.length === 0 ? (
        <div className="empty">Nothing waits for you.</div>
      ) : (
        <div className="mlist">
          {cards.map((c) => (
            <MobileCard
              key={`${c.node_id}/${c.card.id}`}
              view={c}
              columns={board.columns}
              showNode={many}
              offline={nodeById.get(c.node_id)?.online === false}
              actions
              onOpen={() => onOpen(c.node_id, c.card.id)}
              onAction={onAction}
            />
          ))}
        </div>
      )}
    </div>
  );
}
