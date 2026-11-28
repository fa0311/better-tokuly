import { QueryClient, QueryObserver } from "@tanstack/query-core";
import { afterEach, describe, expect, it, vi } from "vitest";
import { reread, rereadOnWelcome, type WelcomingConnection } from "./reread";

/** A connection that says Welcome when told to. */
function connection(status = "connecting") {
  const listeners = new Set<(frame: { body: { case?: string } }) => void>();
  const fake: WelcomingConnection & { welcome(): void } = {
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

/** A query function whose answers the test hands out, one per call. */
function answers() {
  const pending: ((value: string) => void)[] = [];
  return {
    queryFn: () => new Promise<string>((resolve) => pending.push(resolve)),
    calls: () => pending.length,
    answer: (call: number, value: string) => pending[call]?.(value),
  };
}

const watch = (queryClient: QueryClient, key: string[], queryFn: () => unknown) =>
  new QueryObserver(queryClient, {
    queryKey: key,
    queryFn,
    staleTime: Number.POSITIVE_INFINITY,
  }).subscribe(() => {});

describe("reading again", () => {
  it("replaces a first read in flight rather than joining it", async () => {
    const queryClient = new QueryClient();
    const server = answers();
    // A route's loader asks before anyone shows the query: it has no data and no observer.
    const loading = queryClient.fetchQuery({ queryKey: ["sos"], queryFn: server.queryFn });
    expect(server.calls()).toBe(1);

    void reread(queryClient, [["sos"]]);
    expect(server.calls()).toBe(2);
    // The first answer was made before the push the caller missed; it does not count.
    server.answer(0, "before");
    server.answer(1, "after");
    await expect(loading).resolves.toBe("after");
    expect(queryClient.getQueryData(["sos"])).toBe("after");
  });

  it("replaces a read in flight on a query on screen, with or without data", async () => {
    const queryClient = new QueryClient();
    const server = answers();
    const stop = watch(queryClient, ["trips"], server.queryFn);
    void reread(queryClient, [["trips"]]);
    server.answer(0, "before");
    server.answer(1, "after");
    await vi.waitFor(() => expect(queryClient.getQueryData(["trips"])).toBe("after"));

    void queryClient.refetchQueries({ queryKey: ["trips"] });
    void reread(queryClient, [["trips"]]);
    expect(server.calls()).toBe(4);
    server.answer(3, "latest");
    server.answer(2, "older");
    await vi.waitFor(() => expect(queryClient.getQueryState(["trips"])?.fetchStatus).toBe("idle"));
    expect(queryClient.getQueryData(["trips"])).toBe("latest");
    stop();
  });

  it("reads what is on screen at once, marks the rest stale, and leaves covered queries alone", async () => {
    const queryClient = new QueryClient();
    const reads: string[] = [];
    const read = (key: string[]) => () => {
      reads.push(key.join("/"));
      return 1;
    };
    watch(queryClient, ["spaces", "hangouts"], read(["spaces", "hangouts"]));
    watch(queryClient, ["spaces", "space", "covered"], read(["spaces", "space", "covered"]));
    await queryClient.fetchQuery({ queryKey: ["spaces", "list", "mine"], queryFn: () => 1 });
    await vi.waitFor(() => expect(reads).toHaveLength(2));
    reads.length = 0;

    await reread(queryClient, [["spaces"]], (key) => key[1] === "space");
    expect(reads).toEqual(["spaces/hangouts"]);
    expect(queryClient.getQueryState(["spaces", "list", "mine"])?.isInvalidated).toBe(true);
    expect(queryClient.getQueryState(["spaces", "space", "covered"])?.isInvalidated).toBe(false);
  });
});

describe("on every Welcome", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("reads again on the first Welcome at once, and after a reconnect spread out", async () => {
    vi.useFakeTimers();
    const fake = connection();
    const queryClient = new QueryClient();
    let reads = 0;
    watch(queryClient, ["moments"], () => ++reads);
    await vi.runAllTimersAsync();
    expect(reads).toBe(1);

    const stop = rereadOnWelcome(queryClient, fake, [["moments"]]);
    fake.welcome();
    expect(reads).toBe(2);

    vi.spyOn(Math, "random").mockReturnValue(0.5);
    fake.welcome();
    await vi.advanceTimersByTimeAsync(999);
    expect(reads).toBe(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(reads).toBe(3);

    stop();
    fake.welcome();
    await vi.advanceTimersByTimeAsync(2_000);
    expect(reads).toBe(3);
  });

  it("counts a connection that is up already as connected before", async () => {
    vi.useFakeTimers();
    vi.spyOn(Math, "random").mockReturnValue(0.5);
    const fake = connection("connected");
    const queryClient = new QueryClient();
    let reads = 0;
    watch(queryClient, ["plans"], () => ++reads);
    await vi.runAllTimersAsync();
    rereadOnWelcome(queryClient, fake, [["plans"]]);
    fake.welcome();
    expect(reads).toBe(1);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(reads).toBe(2);
  });
});
