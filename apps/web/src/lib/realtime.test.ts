import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";

// The app's connection, as far as the helper needs it.
const fake = vi.hoisted(() => ({
  listeners: new Set<(frame: { body: { case: string } }) => void>(),
  welcome() {
    for (const listener of fake.listeners) listener({ body: { case: "welcome" } });
  },
}));
vi.mock("@tokuly/realtime", () => ({
  RealtimeClient: class {
    connectionStatus = "connecting";
    onFrame(listener: (frame: { body: { case: string } }) => void) {
      fake.listeners.add(listener);
      return () => fake.listeners.delete(listener);
    }
    isSubscribed = (topic: string) => topic === "space:covered";
  },
}));
vi.mock("#/lib/api", () => ({ api: () => ({}) }));
vi.stubGlobal("document", { addEventListener: () => {} });
vi.stubGlobal("window", { localStorage: { getItem: () => "device", setItem: () => {} } });

const { realtime, rereadListsAfterReconnect } = await import("./realtime");

describe("lists read again after a reconnect", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("reads the lists on screen once per reconnect, spread out, and leaves the rest alone", async () => {
    vi.useFakeTimers();
    const queryClient = new QueryClient();
    const reads: string[] = [];
    const watch = (key: string[]) => {
      const observer = new QueryObserver(queryClient, {
        queryKey: key,
        queryFn: () => {
          reads.push(key.join("/"));
          return 1;
        },
        staleTime: Number.POSITIVE_INFINITY,
      });
      return observer.subscribe(() => {});
    };
    watch(["spaces", "hangouts"]);
    watch(["spaces", "space", "covered"]);
    // Read once, then nobody shows it.
    await queryClient.fetchQuery({ queryKey: ["spaces", "list", "mine"], queryFn: () => 1 });
    await vi.runAllTimersAsync();
    reads.length = 0;

    const room = (key: readonly unknown[]) =>
      key[1] === "space" && realtime().isSubscribed(`space:${String(key[2])}`);
    const stop = rereadListsAfterReconnect(queryClient, ["spaces"], room);
    // The first Welcome is no reconnect.
    fake.welcome();
    await vi.advanceTimersByTimeAsync(2_000);
    expect(reads).toEqual([]);

    vi.spyOn(Math, "random").mockReturnValue(0.5);
    fake.welcome();
    await vi.advanceTimersByTimeAsync(999);
    expect(reads).toEqual([]);
    await vi.advanceTimersByTimeAsync(1);
    // The mounted list, not the room its topic covers, not the list nobody shows.
    expect(reads).toEqual(["spaces/hangouts"]);
    expect(queryClient.getQueryState(["spaces", "list", "mine"])?.isInvalidated).toBe(false);

    stop();
    fake.welcome();
    await vi.advanceTimersByTimeAsync(2_000);
    expect(reads).toEqual(["spaces/hangouts"]);
  });
});
