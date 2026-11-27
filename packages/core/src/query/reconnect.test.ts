import { QueryClient, QueryObserver } from "@tanstack/query-core";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  afterReconnect,
  type ReconnectingConnection,
  rereadListsAfterReconnect,
} from "./reconnect";

/** A connection that says Welcome when told to. */
function connection(status = "connecting") {
  const listeners = new Set<(frame: { body: { case?: string } }) => void>();
  const fake: ReconnectingConnection & { welcome(): void } = {
    connectionStatus: status,
    onFrame(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    welcome() {
      for (const listener of listeners) listener({ body: { case: "welcome" } });
    },
  };
  return fake;
}

describe("after a reconnect", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("is told on every Welcome but the first one", () => {
    const fake = connection();
    const told = vi.fn();
    const stop = afterReconnect(fake, told);
    fake.welcome();
    expect(told).not.toHaveBeenCalled();
    fake.welcome();
    fake.welcome();
    expect(told).toHaveBeenCalledTimes(2);
    stop();
    fake.welcome();
    expect(told).toHaveBeenCalledTimes(2);
  });

  it("counts a connection that is up already as the first one", () => {
    const fake = connection("connected");
    const told = vi.fn();
    afterReconnect(fake, told);
    fake.welcome();
    expect(told).toHaveBeenCalledOnce();
  });

  it("reads the lists on screen once per reconnect, spread out, and leaves the rest alone", async () => {
    vi.useFakeTimers();
    const fake = connection();
    const queryClient = new QueryClient();
    const reads: string[] = [];
    const watch = (key: string[]) =>
      new QueryObserver(queryClient, {
        queryKey: key,
        queryFn: () => {
          reads.push(key.join("/"));
          return 1;
        },
        staleTime: Number.POSITIVE_INFINITY,
      }).subscribe(() => {});
    watch(["spaces", "hangouts"]);
    watch(["spaces", "space", "covered"]);
    // Read once, then nobody shows it.
    await queryClient.fetchQuery({ queryKey: ["spaces", "list", "mine"], queryFn: () => 1 });
    await vi.runAllTimersAsync();
    reads.length = 0;

    const stop = rereadListsAfterReconnect(
      queryClient,
      fake,
      ["spaces"],
      (key) => key[1] === "space" && key[2] === "covered",
    );
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
