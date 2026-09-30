import { useEffect, useRef, useState } from "react";
import type { NodeStatus } from "../types";
import { autoHue, HUES, hueIndex, nodeDot, nodeHue, noAutoFill } from "../util";

interface Props {
  /** Every node, in the order the user gave them. */
  nodes: NodeStatus[];
  /** The node the board shows, or "" for all of them. */
  filter: string;
  onFilter: (nodeId: string) => void;
  accountOf: Map<string, string>;
  /** False until the settings have loaded. The edit button waits for them. */
  editable: boolean;
  /** Pick a colour for a node. Null gives the node its own colour back. */
  onHue: (nodeId: string, hue: number | null) => void;
  onRename: (node: NodeStatus, name: string) => void;
  onMove: (nodeId: string, step: -1 | 1) => void;
}

/** The node filter above the board, and the place to name, colour and order
 *  the nodes.
 *
 *  The chips are where the colour of a node is first read, so they are where
 *  it is changed. The edit button turns each chip into its own editor: the dot
 *  opens the palette, the name is a field, and two arrows move the chip. The
 *  order is the order of the chips, and every other list of nodes follows it.
 *
 *  Arrows and not a drag: a chip is small, a drag on it is easy to miss, and
 *  an arrow also works from the keyboard. */
export function NodeChips({ nodes, filter, onFilter, accountOf, editable, onHue, onRename, onMove }: Props) {
  const [editing, setEditing] = useState(false);
  const [palette, setPalette] = useState<string | null>(null);

  if (editing) {
    return (
      <div className="nodechips editing">
        {nodes.map((n, i) => (
          <EditChip
            // A rename that the hub answered, or one made on another screen,
            // starts the field again from the new name.
            key={`${n.id}:${n.name}`}
            node={n}
            first={i === 0}
            last={i === nodes.length - 1}
            paletteOpen={palette === n.id}
            onPalette={() => setPalette(palette === n.id ? null : n.id)}
            onHue={(h) => {
              setPalette(null);
              onHue(n.id, h);
            }}
            onRename={(name) => onRename(n, name)}
            onMove={(step) => onMove(n.id, step)}
          />
        ))}
        <button
          className="btn ghost sm"
          onClick={() => {
            setPalette(null);
            setEditing(false);
          }}
        >
          Done
        </button>
      </div>
    );
  }

  return (
    <div className="nodechips">
      {nodes.length > 1 && (
        <>
          <button className={`nodechip ${filter === "" ? "sel" : ""}`} onClick={() => onFilter("")}>
            All nodes
          </button>
          {nodes.map((n) => (
            <button
              key={n.id}
              // The chip carries the colour of the machine, so the filter
              // reads in the same colour as the cards it keeps.
              className={`nodechip ${nodeHue(n.id)} ${filter === n.id ? "sel" : ""}`}
              title={[
                (n.error ?? (n.enabled ? (n.online ? "online" : "offline") : "disabled")) +
                  (n.pending_writes ? `, ${n.pending_writes} change(s) waiting` : ""),
                accountOf.get(n.id) ? `Account: ${accountOf.get(n.id)}` : "",
              ]
                .filter(Boolean)
                .join("\n")}
              onClick={() => onFilter(filter === n.id ? "" : n.id)}
            >
              <span className={nodeDot(n)} />
              {n.name}
            </button>
          ))}
        </>
      )}
      <button
        className="btn ghost sm nodeedit"
        disabled={!editable}
        onClick={() => setEditing(true)}
        title="Rename, colour and order the nodes"
        aria-label="Edit the nodes"
      >
        ✎
      </button>
    </div>
  );
}

function EditChip({
  node,
  first,
  last,
  paletteOpen,
  onPalette,
  onHue,
  onRename,
  onMove,
}: {
  node: NodeStatus;
  first: boolean;
  last: boolean;
  paletteOpen: boolean;
  onPalette: () => void;
  onHue: (hue: number | null) => void;
  onRename: (name: string) => void;
  onMove: (step: -1 | 1) => void;
}) {
  const [draft, setDraft] = useState(node.name);
  const box = useRef<HTMLSpanElement>(null);
  // A click outside the chip closes its palette.
  useEffect(() => {
    if (!paletteOpen) return;
    const away = (e: PointerEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) onPalette();
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  }, [paletteOpen, onPalette]);

  const commit = () => {
    const name = draft.trim();
    if (name && name !== node.name) onRename(name);
    else setDraft(node.name);
  };
  const current = hueIndex(node.id);
  const own = autoHue(node.id);
  return (
    <span ref={box} className={`nodechip edit ${nodeHue(node.id)}`}>
      <button className="swatch" onClick={onPalette} title="Pick a colour" aria-label={`Colour of ${node.name}`} aria-expanded={paletteOpen}>
        <span className="dot online" />
      </button>
      <input
        {...noAutoFill}
        className="nn"
        value={draft}
        size={Math.max(4, draft.length)}
        aria-label={`Name of ${node.name}`}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") {
            setDraft(node.name);
            e.currentTarget.blur();
          }
        }}
      />
      <button className="mv" disabled={first} onClick={() => onMove(-1)} title="Move left" aria-label={`Move ${node.name} left`}>
        ‹
      </button>
      <button className="mv" disabled={last} onClick={() => onMove(1)} title="Move right" aria-label={`Move ${node.name} right`}>
        ›
      </button>
      {paletteOpen && (
        <span className="palette" role="listbox" aria-label={`Colours for ${node.name}`}>
          {Array.from({ length: HUES }, (_, h) => (
            <button
              key={h}
              role="option"
              aria-selected={h === current}
              className={`sw hue-${h} ${h === current ? "sel" : ""}`}
              // The node's own colour is the one it had before any pick. Picking
              // it clears the choice, so the setting holds only real changes.
              onClick={() => onHue(h === own ? null : h)}
              title={h === own ? "The node's own colour" : `Colour ${h + 1}`}
            />
          ))}
        </span>
      )}
    </span>
  );
}
