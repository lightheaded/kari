import { useEffect, useRef } from "react";

/** The Back button of a phone, and the layers it closes.
 *
 *  Android sends Back to the web view, and the web view goes one step back in
 *  its history. A card that is open adds nothing to that history, so the press
 *  leaves the app instead of closing the card. `useBackClose` puts one history
 *  entry there for each open layer, and takes the entry off again when the
 *  layer closes another way. The history holds one entry per open layer, and
 *  nothing more.
 *
 *  A desktop web view has the same history, so a back gesture there closes the
 *  same layer.
 *
 *  Android also needs the activity to hand the press over. Tauri turns that
 *  off; `scripts/android-back.sh` turns it back on. */

/** The open layers, innermost last. The Back button closes the last one. */
const stack: { close: () => void }[] = [];

/** History entries this module put there. It follows the stack, one behind
 *  while a change waits for `sync`. */
let entries = 0;

/** Steps back this module asked for. The event they raise is not a press. */
let mine = 0;

let timer: number | null = null;

/** Match the history to the stack.
 *
 *  This runs after the current work, not during it, because React mounts an
 *  effect twice in development: the layer goes on the stack, comes off, and
 *  goes on again. Answering each step at once would push an entry and then
 *  step back over the one the second mount pushed, which reads as a press and
 *  closes the card as it opens. A wait of one turn sees the end state only. */
function sync() {
  timer = null;
  while (entries < stack.length) {
    window.history.pushState({ kariLayer: true }, "");
    entries++;
  }
  while (entries > stack.length) {
    entries--;
    mine++;
    window.history.back();
  }
}

function schedule() {
  if (timer === null) timer = window.setTimeout(sync, 0);
}

function onPop() {
  if (mine > 0) {
    mine--;
    return;
  }
  if (entries > 0) entries--;
  // A close that is refused, such as one that asks about unsent text, leaves
  // the layer on the stack. `sync` then puts an entry back, so the next press
  // asks again.
  stack[stack.length - 1]?.close();
  schedule();
}

if (typeof window !== "undefined") window.addEventListener("popstate", onPop);

/** Close this layer when the system Back button is pressed. Call it from a
 *  component that is mounted only while the layer is open. */
export function useBackClose(close: () => void) {
  const latest = useRef(close);
  useEffect(() => {
    latest.current = close;
  });
  useEffect(() => {
    const layer = { close: () => latest.current() };
    stack.push(layer);
    schedule();
    return () => {
      const i = stack.lastIndexOf(layer);
      if (i >= 0) stack.splice(i, 1);
      schedule();
    };
  }, []);
}
