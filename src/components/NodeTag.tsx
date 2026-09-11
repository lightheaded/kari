import { nodeHue } from "../util";

interface Props {
  nodeId: string;
  nodeName: string;
  /** The Claude Code account that pays for the node. Null hides that half. */
  account?: string | null;
  /** False draws the dot as a ring. Undefined means the state is not known. */
  online?: boolean;
  /** A click filters the board to this node. Without it the tag is not a button. */
  onClick?: () => void;
  /** Extra lines under the machine name in the tooltip. */
  title?: string;
  /** Bigger type, for the drawer header. */
  large?: boolean;
}

/** Which machine runs a card, and which account pays for it.
 *
 *  The colour is the point. Every machine holds one colour, from its node id,
 *  and the same colour marks its cards, its dot in the filter bar and its row
 *  in the quota strip. A board with three machines is then readable at arm's
 *  length, which a grey chip with a name on it is not.
 *
 *  The account rides on the same tag rather than on a chip of its own. The
 *  two answer one question — where does this run, and whose quota does it
 *  spend — and two separate chips split that answer over the width of a
 *  card. */
export function NodeTag({ nodeId, nodeName, account, online, onClick, title, large }: Props) {
  const state = online === false ? "offline" : "online";
  const cls = `nodetag ${nodeHue(nodeId)} ${state} ${large ? "lg" : ""} ${onClick ? "act" : ""}`;
  const tip = [
    `Runs on ${nodeName}`,
    account ? `Account: ${account}` : "",
    online === false ? "The node does not answer." : "",
    title ?? "",
    onClick ? "Click to show only the cards of this node." : "",
  ]
    .filter(Boolean)
    .join("\n");
  const body = (
    <>
      <span className="dot" />
      <span className="nn">{nodeName}</span>
      {account && <span className="an">{account}</span>}
    </>
  );
  if (!onClick) {
    return (
      <span className={cls} title={tip}>
        {body}
      </span>
    );
  }
  return (
    <button
      className={cls}
      title={tip}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      // A drag must start on the card, not on this button.
      onPointerDown={(e) => e.stopPropagation()}
    >
      {body}
    </button>
  );
}
