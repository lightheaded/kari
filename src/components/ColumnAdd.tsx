import { useEffect, useRef, useState } from "react";
import { noAutoFill } from "../util";

interface Props {
  columnName: string;
  /** Save a one-line task. Rejects when the node refuses it. */
  onAdd: (title: string) => Promise<void>;
  /** Open the full dialog with what is typed so far. */
  onFull: () => void;
}

/** The foot of every column: add a task here without leaving the board. Enter
 *  saves and keeps the field open for the next one. Escape closes it. */
export function ColumnAdd({ columnName, onAdd, onFull }: Props) {
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
      <div className="draftbar">
        <span className="hint">Enter saves</span>
        <div className="spacer" />
        <button className="btn ghost sm" onClick={onFull} title="Open the full dialog">
          More ⌄
        </button>
        <button className="btn primary sm" disabled={!title.trim() || busy} onClick={save}>
          Add
        </button>
      </div>
    </div>
  );
}
