import type { ServerFrame } from "@tokuly/realtime";
import { describe, expect, it, vi } from "vitest";

const listeners = new Set<(frame: ServerFrame) => void>();
vi.mock("@tokuly/realtime", () => ({
  RealtimeClient: class {
    onFrame(listener: (frame: ServerFrame) => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    }
  },
}));
vi.mock("#/lib/api", () => ({ api: () => ({ session: {} }) }));
vi.mock("#/lib/realtime-identity", () => ({ restartWhenSessionChanges: () => {} }));
vi.stubGlobal("window", { localStorage: { getItem: () => "device", setItem: () => {} } });
vi.stubGlobal("document", { addEventListener: () => {} });

const { onFramesLoading } = await import("./realtime");

const push = (body: ServerFrame["body"]) => {
  for (const listener of [...listeners]) listener({ replyTo: 0n, topic: "", body });
};
const presence = (userId: string) =>
  ({ case: "presence", value: { userId, online: true } }) as unknown as ServerFrame["body"];

/** A load that finishes when the test says so. */
function held() {
  let finish = (_ok: boolean) => {};
  const loading = () =>
    new Promise<(value: { userId: string }) => void>((resolve, reject) => {
      finish = (ok) =>
        ok ? resolve((value) => seen.push(value.userId)) : reject(new Error("gone"));
    });
  const seen: string[] = [];
  return { loading, seen, finish: (ok: boolean) => finish(ok) };
}

describe("onFramesLoading", () => {
  it("loads with the first frame of its case and hands on the waiting ones in order", async () => {
    const load = held();
    const spy = vi.fn(load.loading);
    const off = onFramesLoading("presence", spy);
    push({ case: "pong", value: {} } as unknown as ServerFrame["body"]);
    expect(spy).not.toHaveBeenCalled();
    push(presence("a"));
    push(presence("b"));
    expect(spy).toHaveBeenCalledTimes(1);
    expect(load.seen).toEqual([]);
    load.finish(true);
    await vi.waitFor(() => expect(load.seen).toEqual(["a", "b"]));
    push(presence("c"));
    expect(load.seen).toEqual(["a", "b", "c"]);
    off();
  });

  it("loses the frames of a failed load and tries again with the next", async () => {
    const first = held();
    const second = held();
    const spy = vi
      .fn()
      .mockImplementationOnce(first.loading)
      .mockImplementationOnce(second.loading);
    const off = onFramesLoading("presence", spy);
    push(presence("lost"));
    first.finish(false);
    await vi.waitFor(() => expect(spy).toHaveBeenCalledTimes(1));
    await Promise.resolve();
    push(presence("kept"));
    expect(spy).toHaveBeenCalledTimes(2);
    second.finish(true);
    await vi.waitFor(() => expect(second.seen).toEqual(["kept"]));
    off();
  });

  it("hands nothing on once unsubscribed while loading", async () => {
    const load = held();
    const off = onFramesLoading("presence", load.loading);
    push(presence("a"));
    off();
    load.finish(true);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(load.seen).toEqual([]);
  });
});
