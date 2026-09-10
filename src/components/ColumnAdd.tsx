import { useEffect, useRef, useState } from "react";
import { noAutoFill } from "../util";

/** Where the line that is typed now will land, as the draft bar shows it. */
export interface AddPreview {
  /** The node name, or an empty string when the board has one node. */
  node: string;
  /** The project name, or null when nothing names one. */
  project: string | null;
  /** A tag that names no project, without the `#`. Empty when there is none. */
  unknown: string;
}

interface Props {
  columnName: string;
  /** Read the draft. The answer holds the node, the project and a tag that
   *  missed. It is read on every keystroke, so the card never goes unseen. */
  preview: (title: string) => AddPreview;
  /** Save a one-line task. Rejects when the node refuses it. */
  onAdd: (title: string) => Promise<void>;
  /** Open the full dialog with what is typed so far. */
  onFull: (title: string) => void;
}

/** The foot of every column: add a task here without leaving the board. Enter
 *  saves and keeps the field open for the next one. Escape closes it. A
 *  `#project` word in the line picks the project and leaves the title. */
export function ColumnAdd({ columnName, preview, onAdd, onFull }: Props) {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);
  const area = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    if (open) area.current?.focus();
  }, [open]);

  const save = async () => {
    const t = title.trim();
    if (!t || busy) return;
    setBusy(true);
    try {
      await onAdd(t);
      setTitle("");
      area.current?.focus();
    } finally {
      setBusy(false);
    }
  };

  if (!open) {
    return (
      <button className="coladd" onClick={() => setOpen(true)} title={`Add a task to ${columnName}`}>
        + Add task
      </button>
    );
  }

  const at = preview(title);

  return (
    <div className="coldraft">
      <textarea
        {...noAutoFill}
        ref={area}
        rows={2}
        value={title}
        disabled={busy}
        placeholder="What needs to happen"
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            save();
          } else if (e.key === "Escape") {
            e.preventDefault();
            setOpen(false);
            setTitle("");
          }
        }}
        onBlur={() => {
          if (!title.trim()) setOpen(false);
        }}
      />
      <div
        className={`coltarget hint ${at.unknown ? "miss" : ""}`}
        title={
          at.unknown
            ? `No project is called ${at.unknown}. The word stays in the title.`
            : "Type #name to pick a project. Change either of these in the full dialog, or later on the card"
        }
      >
        {at.project ? `→ ${at.project}` : "→ no project yet"}
        {at.node ? ` · ${at.node}` : ""}
        {at.unknown ? ` · #${at.unknown} names no project` : ""}
      </div>
      <div className="draftbar">
        <span className="hint">Enter saves · #name picks a project</span>
        <div className="spacer" />
        <button className="btn ghost sm" onClick={() => onFull(title)} title="Open the full dialog">
          More ⌄
        </button>
        <button className="btn primary sm" disabled={!title.trim() || busy} onClick={save}>
          Add
        </button>
      </div>
    </div>
  );
}
