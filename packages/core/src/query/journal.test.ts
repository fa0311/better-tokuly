import { QueryClient } from "@tanstack/query-core";
import { describe, expect, it } from "vitest";
import { journaled, pushTo } from "./journal";
import { reread } from "./reread";

interface Line {
  id: string;
  version: number;
}

const key = ["chat", "room"];
// Idempotent, as pushed updates must be: by ID, the higher version wins.
const add = (line: Line) => (lines: Line[]) => {
  const held = lines.find((other) => other.id === line.id);
  if (held && held.version >= line.version) return lines;
  return [...lines.filter((other) => other.id !== line.id), line];
};
const remove = (id: string) => (lines: Line[]) => lines.filter((line) => line.id !== id);

/** A server whose answers the test hands out, one per read. */
function server() {
  const pending: ((lines: Line[]) => void)[] = [];
  return {
    queryFn: journaled(() => new Promise<Line[]>((resolve) => pending.push(resolve))),
    answer: (read: number, lines: Line[]) => pending[read]?.(lines),
  };
}

describe("journaled reads and pushes", () => {
  it("applies a push made while a read was in flight to its answer", async () => {
    const queryClient = new QueryClient();
    const api = server();
    const loading = queryClient.fetchQuery({ queryKey: key, queryFn: api.queryFn });
    // Nothing is cached yet: the push waits for the answer.
    pushTo(queryClient, key, add({ id: "pushed", version: 1 }));
    api.answer(0, [{ id: "old", version: 1 }]);
    await loading;
    expect(queryClient.getQueryData(key)).toEqual([
      { id: "old", version: 1 },
      { id: "pushed", version: 1 },
    ]);
  });

  it("does not let a read undo a removal pushed while it was in flight", async () => {
    const queryClient = new QueryClient();
    const api = server();
    queryClient.setQueryData<Line[]>(key, [{ id: "gone", version: 1 }]);
    const loading = queryClient.fetchQuery({ queryKey: key, queryFn: api.queryFn });
    pushTo<Line[]>(queryClient, key, remove("gone"));
    expect(queryClient.getQueryData(key)).toEqual([]);
    // The answer was made before the removal.
    api.answer(0, [{ id: "gone", version: 1 }]);
    await loading;
    expect(queryClient.getQueryData(key)).toEqual([]);
  });

  it("does not let an older answer overwrite a newer push, nor replay a push on a later read", async () => {
    const queryClient = new QueryClient();
    const api = server();
    const first = queryClient.fetchQuery({ queryKey: key, queryFn: api.queryFn });
    pushTo(queryClient, key, add({ id: "a", version: 2 }));
    api.answer(0, [{ id: "a", version: 1 }]);
    await first;
    expect(queryClient.getQueryData(key)).toEqual([{ id: "a", version: 2 }]);

    // A read that starts after the push has it, or knows better: the push is not applied again.
    const second = queryClient.fetchQuery({ queryKey: key, queryFn: api.queryFn, staleTime: 0 });
    api.answer(1, []);
    await second;
    expect(queryClient.getQueryData(key)).toEqual([]);
  });

  it("keeps pushes for the read that replaced one in flight, and only for that key", async () => {
    const queryClient = new QueryClient();
    const api = server();
    const other = ["chat", "other"];
    const loading = queryClient.fetchQuery({ queryKey: key, queryFn: api.queryFn });
    queryClient.setQueryData<Line[]>(other, []);
    void reread(queryClient, [key]);
    pushTo(queryClient, key, add({ id: "pushed", version: 1 }));
    pushTo<Line[]>(queryClient, other, add({ id: "elsewhere", version: 1 }));
    api.answer(0, []);
    api.answer(1, [{ id: "read", version: 1 }]);
    await expect(loading).resolves.toEqual([
      { id: "read", version: 1 },
      { id: "pushed", version: 1 },
    ]);
    expect(queryClient.getQueryData(other)).toEqual([{ id: "elsewhere", version: 1 }]);
  });

  it("leaves an empty answer empty, and forgets a read that failed", async () => {
    const queryClient = new QueryClient();
    let fail = true;
    const queryFn = journaled<Line[] | null>(async () => {
      if (fail) throw new Error("down");
      return null;
    });
    await expect(
      queryClient.fetchQuery({ queryKey: key, queryFn, retry: false }),
    ).rejects.toThrow();
    pushTo<Line[]>(queryClient, key, add({ id: "lost", version: 1 }));
    fail = false;
    await queryClient.fetchQuery({ queryKey: key, queryFn, staleTime: 0 });
    expect(queryClient.getQueryData(key)).toBeNull();
  });

  it("does not let a read bring back what a push said is gone", async () => {
    const queryClient = new QueryClient();
    const api = server();
    queryClient.setQueryData<Line[] | null>(key, [{ id: "widget", version: 1 }]);
    const loading = queryClient.fetchQuery({ queryKey: key, queryFn: api.queryFn });
    pushTo<Line[] | null>(queryClient, key, () => null);
    expect(queryClient.getQueryData(key)).toBeNull();
    // Answered before it was revoked; a later push on the empty query changes nothing.
    pushTo<Line[] | null>(queryClient, key, add({ id: "late", version: 1 }));
    api.answer(0, [{ id: "widget", version: 1 }]);
    await loading;
    expect(queryClient.getQueryData(key)).toBeNull();
  });

  it("refuses to journal one page of an infinite query", async () => {
    const queryClient = new QueryClient();
    await expect(
      queryClient.fetchInfiniteQuery({
        queryKey: key,
        // Typed apart on purpose: nothing but this check stops it at run time.
        queryFn: journaled(async () => [] as Line[]) as never,
        initialPageParam: 0,
        getNextPageParam: () => undefined,
        retry: false,
      }),
    ).rejects.toThrow(/not a page/);
  });
});
