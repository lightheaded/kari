import { useCallback, useEffect, useState } from "react";
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
