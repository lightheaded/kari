import { beforeEach, expect, test } from "bun:test";
import { dismissSend, pendingSends, trackSend } from "./pending";

beforeEach(() => {
  for (const p of pendingSends()) dismissSend(p.id);
});

test("a send shows at once and keeps what the node said", async () => {
  let done!: (v: string) => void;
  const sent = trackSend("n", "c", "push it", () => new Promise<string>((r) => (done = r)), () => true);
  expect(pendingSends().map((p) => p.state)).toEqual(["sending"]);
  done("Held for approval in that session (pid 1).");
  await sent;
  expect(pendingSends()[0]).toMatchObject({ state: "sent", text: "push it", note: "Held for approval in that session (pid 1)." });
});

test("a failed send leaves the screen when the box takes the text back", async () => {
  let back = "";
  const restore = (t: string) => {
    back = t;
    return true;
  };
  await expect(trackSend("n", "c", "push it", () => Promise.reject(new Error("offline")), restore)).rejects.toThrow("offline");
  expect(back).toBe("push it");
  expect(pendingSends()).toEqual([]);
});

test("a failed send stays with its error when the box holds new text", async () => {
  await expect(trackSend("n", "c", "push it", () => Promise.reject(new Error("offline")), () => false)).rejects.toThrow();
  expect(pendingSends()[0]).toMatchObject({ state: "failed", note: "Error: offline" });
});
