import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "./api";

// One counter for the whole window: how many forms hold unsaved input.
// The Rust side reads it before a quit from the tray or Cmd+Q.
let count = 0;

function sync() {
  api.setDirty(count > 0).catch(() => {});
}

export function anyDirty(): boolean {
  return count > 0;
}

/** Register unsaved input while `flag` is true. */
export function useDirty(flag: boolean) {
  useEffect(() => {
    if (!flag) return;
    count++;
    sync();
    return () => {
      count--;
      sync();
    };
  }, [flag]);
}

export interface CloseGuard {
  /** The first close attempt on a dirty form sets this. The bar with Keep and Discard shows. */
  asking: boolean;
  /** Close, or ask first when the form is dirty. A second call while asking closes. */
  requestClose: () => void;
  keep: () => void;
  discard: () => void;
}

/**
 * Escape, the backdrop, and Cancel all go through `requestClose`. With unsaved
 * input the first attempt asks, the second one discards. This is what saves a
 * typed task from one stray Escape.
 */
export function useCloseGuard(dirty: boolean, onClose: () => void): CloseGuard {
  const [asked, setAsking] = useState(false);
  // A form that became clean again (the user emptied it) needs no question.
  const asking = asked && dirty;
  useDirty(dirty);
  const requestClose = useCallback(() => {
    if (dirty && !asking) setAsking(true);
    else onClose();
  }, [dirty, asking, onClose]);
  const keep = useCallback(() => setAsking(false), []);
  return { asking, requestClose, keep, discard: onClose };
}

// ---------------------------------------------------------------- drafts

const PREFIX = "kari.draft.";

export interface Draft {
  /** Drop the draft. Call it after a save, and when the user throws the input away. */
  clear: () => void;
  /** A saved draft went back into the form when it opened. */
  restored: boolean;
}

/**
 * Keep the input of a form in the browser store, so a restart, a crash, or a
 * kill that no question can catch does not take it away.
 *
 * `fields` holds the current input. `restore` puts a saved copy back when the
 * form opens. The draft goes away when the form is clean again, and when the
 * returned `clear` runs after a save.
 *
 * `key` must name the form and, when the form edits one card, that card.
 */
export function useDraft<T>(key: string, dirty: boolean, fields: T, restore: (saved: T) => void): Draft {
  const name = PREFIX + key;
  // The caller builds a new function on every render. Only the last one counts,
  // and it must be in place before the effect below reads a draft.
  const restoreRef = useRef(restore);
  useEffect(() => {
    restoreRef.current = restore;
  });
  // The key this form last wrote a draft under. A clean form drops only the
  // draft it wrote itself, so an untouched form never drops a stored one.
  const saved = useRef<string | null>(null);
  const clear = useCallback(() => {
    try {
      window.localStorage.removeItem(name);
      saved.current = null;
    } catch {
      // no storage: nothing was written
    }
  }, [name]);

  // Which key the form was filled from. A change of card makes it stale, and
  // the flag then reads false again with no write of its own.
  const [filledFrom, setFilledFrom] = useState<string | null>(null);
  useEffect(() => {
    try {
      const raw = window.localStorage.getItem(name);
      if (raw === null) return;
      restoreRef.current(JSON.parse(raw) as T);
      setFilledFrom(name);
    } catch {
      // no storage, or a draft this version cannot read
    }
  }, [name]);

  const open = useRef<string | null>(null);
  const json = JSON.stringify(fields);
  useEffect(() => {
    // The pass that opens a key runs before `restore` reaches the state, and
    // the fields still belong to the form as it was. Write nothing yet.
    if (open.current !== name) {
      open.current = name;
      return;
    }
    try {
      if (dirty) {
        window.localStorage.setItem(name, json);
        saved.current = name;
      } else if (saved.current === name) {
        window.localStorage.removeItem(name);
        saved.current = null;
      }
    } catch {
      // no storage: the input holds for this run only
    }
  }, [name, dirty, json]);

  return { clear, restored: filledFrom === name };
}
