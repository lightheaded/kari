import { useEffect, useRef, useState } from "react";
import type { Picked } from "./Board";
import type { Toast, Undo } from "../toasts";

interface Props {
  toasts: Toast[];
  onDrop: (id: number) => void;
  onClear: () => void;
  onOpen?: (card: Picked) => void;
  onUndo?: (u: Undo) => void;
}

/** The stack in the corner. The whole stack holds while the pointer is on it,
 *  or while a button in it has the focus, so nothing moves under the hand and
 *  a long notice can be read to the end. */
export function Toasts({ toasts, onDrop, onClear, onOpen, onUndo }: Props) {
  const [held, setHeld] = useState(false);
  return (
    <div
      className="toasts"
      onMouseEnter={() => setHeld(true)}
      onMouseLeave={() => setHeld(false)}
      onFocusCapture={() => setHeld(true)}
      onBlurCapture={() => setHeld(false)}
    >
      {toasts.length > 1 && (
        <button className="dismiss-all" onClick={onClear}>
          Dismiss all
        </button>
      )}
      {toasts.map((t) => (
        <ToastItem
          key={t.id}
          t={t}
          held={held}
          onClose={() => onDrop(t.id)}
          onOpen={
            t.card && onOpen
              ? () => {
                  onOpen(t.card!);
                  onDrop(t.id);
                }
              : undefined
          }
          onUndo={
            t.undo && onUndo
              ? () => {
                  onDrop(t.id);
                  onUndo(t.undo!);
                }
              : undefined
          }
        />
      ))}
    </div>
  );
}

interface ItemProps {
  t: Toast;
  held: boolean;
  onClose: () => void;
  onOpen?: () => void;
  onUndo?: () => void;
}

/** One toast. The shrinking bar along the foot is the clock: it runs for the
 *  life of the toast, and the toast closes when it ends. So a hold stops both
 *  at once, and the bar always shows the time that is left. */
function ToastItem({ t, held, onClose, onOpen, onUndo }: ItemProps) {
  const bar = useRef<HTMLElement | null>(null);
  const anim = useRef<Animation | null>(null);
  // The parent gives a new close callback on every render. Keep the newest one
  // in a ref, so a render cannot restart the clock.
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  });

  useEffect(() => {
    const el = bar.current;
    // No animation engine, for example in a test renderer: fall back to a timer.
    if (!el || typeof el.animate !== "function") {
      const id = window.setTimeout(() => close.current(), t.ttl);
      return () => window.clearTimeout(id);
    }
    const a = el.animate([{ transform: "scaleX(1)" }, { transform: "scaleX(0)" }], {
      duration: t.ttl,
      easing: "linear",
      fill: "forwards",
    });
    a.onfinish = () => close.current();
    anim.current = a;
    return () => {
      a.onfinish = null;
      a.cancel();
      anim.current = null;
    };
  }, [t.ttl]);

  useEffect(() => {
    const a = anim.current;
    if (!a) return;
    if (held) a.pause();
    else a.play();
  }, [held]);

  return (
    <div className={`toast ${t.err ? "err" : ""} ${onOpen ? "link" : ""}`} onClick={onOpen} role="status">
      <span className="msg">{t.text}</span>
      {onUndo && (
        <button
          className="undo"
          onClick={(e) => {
            e.stopPropagation();
            onUndo();
          }}
        >
          Undo
        </button>
      )}
      <button
        className="x"
        aria-label="Dismiss"
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
      >
        ✕
      </button>
      <i className="life" ref={bar} />
    </div>
  );
}
