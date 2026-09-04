import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/** Read a value from localStorage once, and write it back on every change.
 *  A browser without storage keeps the value for this run only. */
export function useSticky<T>(key: string, initial: T): [T, (v: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = window.localStorage.getItem(key);
      return raw === null ? initial : (JSON.parse(raw) as T);
    } catch {
      return initial;
    }
  });
  const set = useCallback(
    (v: T) => {
      setValue(v);
      try {
        window.localStorage.setItem(key, JSON.stringify(v));
      } catch {
        // no storage: the value holds for this run
      }
    },
    [key],
  );
  return [value, set];
}

/** The height of a textarea, in pixels, that the user dragged. */
const HEIGHT_KEY = "kari.textareaHeight";

function readHeights(): Record<string, number> {
  try {
    return JSON.parse(window.localStorage.getItem(HEIGHT_KEY) ?? "{}");
  } catch {
    return {};
  }
}

function writeHeight(name: string, px: number) {
  try {
    const all = readHeights();
    all[name] = px;
    window.localStorage.setItem(HEIGHT_KEY, JSON.stringify(all));
  } catch {
    // no storage: the height holds for this run
  }
}

/** Grow a textarea to fit its text, up to `max` pixels, and keep a height the
 *  user dragged. Returns the props to spread on the textarea.
 *
 *  `name` identifies the field across openings of the drawer or the dialog.
 *  A drag beats the content: once the user sets a height, that height stays
 *  until the text needs more room than it gives. */
export function useAutoGrow(name: string, value: string, min = 68, max = 420) {
  const ref = useRef<HTMLTextAreaElement | null>(null);
  const dragged = useRef<number | null>(readHeights()[name] ?? null);

  const fit = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    // Measure the text with no height of its own in the way.
    el.style.height = "auto";
    const needed = el.scrollHeight + el.offsetHeight - el.clientHeight;
    const floor = Math.max(min, dragged.current ?? min);
    el.style.height = `${Math.min(max, Math.max(floor, needed))}px`;
  }, [min, max]);

  useLayoutEffect(fit, [fit, value, name]);

  // A drag changes the element height without any event of its own.
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    let last = el.offsetHeight;
    const ro = new ResizeObserver(() => {
      const h = el.offsetHeight;
      // Ignore the changes `fit` makes: only a pointer drag counts.
      if (Math.abs(h - last) > 2 && document.activeElement === el) {
        dragged.current = h;
        writeHeight(name, h);
      }
      last = h;
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [name]);

  return { ref, onInput: fit };
}
