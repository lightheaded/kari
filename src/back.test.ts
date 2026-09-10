import { describe, expect, test } from "bun:test";
import { createBackStack, type BackHost } from "./back";

/** A history of counted entries, and a back press that steps through them.
 *  A press with no entry left does nothing here, as it leaves the app there. */
function fakeHistory() {
  let listeners: (() => void)[] = [];
  let entries = 0;
  const host: BackHost = {
    push: () => {
      entries++;
    },
    back: () => {
      if (entries === 0) return;
      entries--;
      for (const cb of [...listeners]) cb();
    },
    listen: (cb) => {
      listeners.push(cb);
      return () => {
        listeners = listeners.filter((x) => x !== cb);
      };
    },
  };
  return { host, entries: () => entries, press: () => host.back() };
}

describe("createBackStack", () => {
  test("a back press closes the layer that pushed", () => {
    const h = fakeHistory();
    const stack = createBackStack(h.host);
    let closed = 0;
    stack.push(() => closed++);
    expect(h.entries()).toBe(1);
    expect(stack.depth()).toBe(1);
    h.press();
    expect(closed).toBe(1);
    expect(stack.depth()).toBe(0);
    expect(h.entries()).toBe(0);
  });

  test("a layer closed from the code takes its entry with it", async () => {
    const h = fakeHistory();
    const stack = createBackStack(h.host);
    let closed = 0;
    const release = stack.push(() => closed++);
    release();
    await null;
    expect(stack.depth()).toBe(0);
    expect(h.entries()).toBe(0);
    // The layer is gone, so the press has nothing to close and leaves the app.
    h.press();
    expect(closed).toBe(0);
  });

  test("a second release closes nothing twice", async () => {
    const h = fakeHistory();
    const stack = createBackStack(h.host);
    const release = stack.push(() => {});
    release();
    release();
    await null;
    expect(h.entries()).toBe(0);
  });

  test("a layer that opens as another closes takes the entry over", async () => {
    const h = fakeHistory();
    const stack = createBackStack(h.host);
    const said: string[] = [];
    // One tick, two layers: the first closes and the second opens. This is
    // what React does when it mounts a component twice to check it.
    const release = stack.push(() => said.push("first"));
    release();
    stack.push(() => said.push("second"));
    await null;
    expect(stack.depth()).toBe(1);
    expect(h.entries()).toBe(1);
    // One press, and the layer that is open closes.
    h.press();
    expect(said).toEqual(["second"]);
    expect(h.entries()).toBe(0);
  });

  test("a press closes the top layer only", () => {
    const h = fakeHistory();
    const stack = createBackStack(h.host);
    const said: string[] = [];
    stack.push(() => said.push("sheet"));
    stack.push(() => said.push("dialog"));
    expect(h.entries()).toBe(2);
    h.press();
    expect(said).toEqual(["dialog"]);
    h.press();
    expect(said).toEqual(["dialog", "sheet"]);
    expect(h.entries()).toBe(0);
  });

  test("the entries still match when a layer under the top closes itself", async () => {
    const h = fakeHistory();
    const stack = createBackStack(h.host);
    const said: string[] = [];
    const releaseUnder = stack.push(() => said.push("under"));
    stack.push(() => said.push("over"));
    releaseUnder();
    await null;
    expect(stack.depth()).toBe(1);
    expect(h.entries()).toBe(1);
    h.press();
    expect(said).toEqual(["over"]);
    expect(h.entries()).toBe(0);
  });

  test("a press after every layer closed leaves the app", () => {
    const h = fakeHistory();
    const stack = createBackStack(h.host);
    let closed = 0;
    stack.push(() => closed++);
    h.press();
    h.press();
    expect(closed).toBe(1);
    expect(h.entries()).toBe(0);
  });
});
