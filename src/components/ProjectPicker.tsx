import { useEffect, useMemo, useRef, useState } from "react";
import { fuzzyScore, noAutoFill } from "../util";

export interface PickerItem {
  /** The value the caller stores. */
  value: string;
  /** What the user reads. */
  label: string;
  /** A second line, such as the node or the path. */
  hint?: string;
}

interface Props {
  items: PickerItem[];
  value: string;
  /** Shown when nothing is chosen. Choosing it passes an empty value back. */
  allLabel?: string;
  placeholder?: string;
  ariaLabel: string;
  onChange: (value: string) => void;
}

/** A combobox with fuzzy search over the items. Every character of the query
 *  must appear in order in the label or the hint. Arrow keys walk the list,
 *  Enter takes the top match, Escape closes it. */
export function ProjectPicker({ items, value, allLabel, placeholder, ariaLabel, onChange }: Props) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [at, setAt] = useState(0);
  const box = useRef<HTMLDivElement | null>(null);
  const input = useRef<HTMLInputElement | null>(null);

  const chosen = items.find((i) => i.value === value);
  const shown = chosen?.label ?? allLabel ?? "";

  const matches = useMemo(() => {
    const all = allLabel ? [{ value: "", label: allLabel }, ...items] : items;
    if (!query.trim()) return all;
    return all
      .map((i) => ({ i, s: Math.max(fuzzyScore(i.label, query), fuzzyScore(i.hint ?? "", query) * 0.6) }))
      .filter((x) => x.s > 0)
      .sort((a, b) => b.s - a.s)
      .map((x) => x.i);
  }, [items, query, allLabel]);

  // A click anywhere else closes the list.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  const take = (v: string) => {
    onChange(v);
    setOpen(false);
    setQuery("");
  };

  return (
    <div className={`picker ${open ? "open" : ""}`} ref={box}>
      <input
        {...noAutoFill}
        ref={input}
        className="pickerbtn"
        role="combobox"
        aria-expanded={open}
        aria-label={ariaLabel}
        placeholder={placeholder ?? shown}
        value={open ? query : shown}
        onFocus={() => {
          setOpen(true);
          setQuery("");
          setAt(0);
        }}
        onChange={(e) => {
          setQuery(e.target.value);
          setAt(0);
          setOpen(true);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setOpen(true);
            setAt((i) => Math.min(matches.length - 1, i + 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setAt((i) => Math.max(0, i - 1));
          } else if (e.key === "Enter") {
            e.preventDefault();
            const pick = matches[at];
            if (pick) take(pick.value);
          } else if (e.key === "Escape") {
            e.preventDefault();
            setOpen(false);
            setQuery("");
            input.current?.blur();
          }
        }}
      />
      {open && (
        <div className="pickerlist" role="listbox">
          {matches.length === 0 && <div className="pickernone">nothing matches</div>}
          {matches.slice(0, 40).map((i, k) => (
            <button
              key={i.value || "__all"}
              role="option"
              aria-selected={k === at}
              className={`pickeropt ${k === at ? "at" : ""} ${i.value === value ? "sel" : ""}`}
              onMouseEnter={() => setAt(k)}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => take(i.value)}
            >
              <span className="l">{i.label}</span>
              {i.hint && <span className="h">{i.hint}</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
