import { type InfiniteData, QueryClient } from "@tanstack/query-core";
import { describe, expect, it } from "vitest";
import { journal, pushAfter, pushTo, pushToAll } from "./journal";
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

/** A server whose answers the test hands out, one per read; `read` is the journaled query function. */
function server() {
  const pending: ((lines: Line[]) => void)[] = [];
  const signals: AbortSignal[] = [];
  return {
    read: {
      queryFn: ({ signal }: { signal: AbortSignal }) => {
        signals.push(signal);
        return new Promise<Line[]>((resolve) => pending.push(resolve));
      },
      persister: journal,
    },
    answer: (read: number, lines: Line[]) => pending[read]?.(lines),
    signals,
  };
}

describe("journaled reads and pushes", () => {
  it("applies a push made while a read was in flight to its answer", async () => {
    const queryClient = new QueryClient();
    const api = server();
    const loading = queryClient.fetchQuery({ queryKey: key, ...api.read });
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
    const loading = queryClient.fetchQuery({ queryKey: key, ...api.read });
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
    const first = queryClient.fetchQuery({ queryKey: key, ...api.read });
    pushTo(queryClient, key, add({ id: "a", version: 2 }));
    api.answer(0, [{ id: "a", version: 1 }]);
    await first;
    expect(queryClient.getQueryData(key)).toEqual([{ id: "a", version: 2 }]);

    // A read that starts after the push has it, or knows better: the push is not applied again.
    const second = queryClient.fetchQuery({ queryKey: key, ...api.read, staleTime: 0 });
    api.answer(1, []);
    await second;
    expect(queryClient.getQueryData(key)).toEqual([]);
  });

  it("keeps pushes for the read that replaced one in flight, and only for that key", async () => {
    const queryClient = new QueryClient();
    const api = server();
    const other = ["chat", "other"];
    const loading = queryClient.fetchQuery({ queryKey: key, ...api.read });
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
    const read = {
      queryFn: async (): Promise<Line[] | null> => {
        if (fail) throw new Error("down");
        return null;
      },
      persister: journal,
    };
    await expect(
      queryClient.fetchQuery({ queryKey: key, ...read, retry: false }),
    ).rejects.toThrow();
    pushTo<Line[]>(queryClient, key, add({ id: "lost", version: 1 }));
    fail = false;
    await queryClient.fetchQuery({ queryKey: key, ...read, staleTime: 0 });
    expect(queryClient.getQueryData(key)).toBeNull();
  });

  it("does not let a read bring back what a push said is gone", async () => {
    const queryClient = new QueryClient();
    const api = server();
    queryClient.setQueryData<Line[] | null>(key, [{ id: "widget", version: 1 }]);
    const loading = queryClient.fetchQuery({ queryKey: key, ...api.read });
    pushTo<Line[] | null>(queryClient, key, () => null);
    expect(queryClient.getQueryData(key)).toBeNull();
    // Answered before it was revoked; a later push on the empty query changes nothing.
    pushTo<Line[] | null>(queryClient, key, add({ id: "late", version: 1 }));
    api.answer(0, [{ id: "widget", version: 1 }]);
    await loading;
    expect(queryClient.getQueryData(key)).toBeNull();
  });

  it("releases a read that was replaced, and tells its request to stop", async () => {
    const queryClient = new QueryClient();
    const api = server();
    const loading = queryClient.fetchQuery({ queryKey: key, ...api.read });
    void reread(queryClient, [key]);
    expect(api.signals[0]?.aborted).toBe(true);
    expect(api.signals[1]?.aborted).toBe(false);
    pushTo(queryClient, key, add({ id: "pushed", version: 1 }));
    api.answer(1, []);
    await expect(loading).resolves.toEqual([{ id: "pushed", version: 1 }]);
  });
});

type Pages = InfiniteData<Line[], number>;
const onPages = (update: (lines: Line[]) => Line[]) => (data: Pages) => ({
  ...data,
  pages: data.pages.map(update),
});

describe("infinite queries", () => {
  /** A list in pages of what the test answers; page `n` is read with `pageParam` n. */
  function pagedServer() {
    const pending: ((lines: Line[]) => void)[] = [];
    return {
      options: {
        queryKey: key,
        queryFn: () => new Promise<Line[]>((resolve) => pending.push(resolve)),
        persister: journal,
        initialPageParam: 0,
        getNextPageParam: (_last: Line[], pages: Line[][]) => pages.length,
      },
      answer: (read: number, lines: Line[]) => pending[read]?.(lines),
    };
  }
  const ids = (data: Pages | undefined) => data?.pages.map((page) => page.map((line) => line.id));

  it("applies a push made while every page was read again to all of them", async () => {
    const queryClient = new QueryClient();
    const api = pagedServer();
    queryClient.setQueryData<Pages>(key, {
      pages: [[{ id: "a", version: 1 }], [{ id: "doomed", version: 1 }]],
      pageParams: [0, 1],
    });
    const loading = queryClient.fetchInfiniteQuery({ ...api.options, staleTime: 0 });
    await Promise.resolve();
    api.answer(0, [{ id: "a", version: 1 }]);
    await new Promise((resolve) => setTimeout(resolve));
    // The first page is in; the second is still on its way.
    pushTo<Pages>(queryClient, key, onPages(remove("doomed")));
    api.answer(1, [{ id: "doomed", version: 1 }]);
    await loading;
    expect(ids(queryClient.getQueryData<Pages>(key))).toEqual([["a"], []]);
  });

  it("does not lose a push made while the next page was read", async () => {
    const queryClient = new QueryClient();
    const api = pagedServer();
    const first = queryClient.fetchInfiniteQuery(api.options);
    await Promise.resolve();
    api.answer(0, [{ id: "a", version: 1 }]);
    await first;
    const query = queryClient.getQueryCache().find({ queryKey: key });
    const next = query?.fetch(undefined, { meta: { fetchMore: { direction: "forward" } } });
    // The next page is built on the pages held when it started: the push lands meanwhile.
    pushTo<Pages>(
      queryClient,
      key,
      onPages((lines) => lines.map((line) => (line.id === "a" ? { id: "a", version: 2 } : line))),
    );
    await Promise.resolve();
    api.answer(1, [{ id: "b", version: 1 }]);
    await next;
    const data = queryClient.getQueryData<Pages>(key);
    expect(ids(data)).toEqual([["a"], ["b"]]);
    expect(data?.pages[0]?.[0]?.version).toBe(2);
  });
});

describe("one list under several keys", () => {
  it("pushes to every query under the prefix, and to their reads in flight", async () => {
    const queryClient = new QueryClient();
    const api = server();
    queryClient.setQueryData<Line[]>(["chat", "list", "inbox"], []);
    const archived = queryClient.fetchQuery({
      queryKey: ["chat", "list", "archived"],
      ...api.read,
    });
    pushToAll<Line[]>(queryClient, ["chat", "list"], add({ id: "new", version: 1 }));
    api.answer(0, []);
    await archived;
    expect(queryClient.getQueryData(["chat", "list", "inbox"])).toEqual([
      { id: "new", version: 1 },
    ]);
    expect(queryClient.getQueryData(["chat", "list", "archived"])).toEqual([
      { id: "new", version: 1 },
    ]);
  });
});

describe("a read that is not the query's own", () => {
  it("adds what it read, then what was pushed since: a removal that followed stays", async () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData<Line[]>(key, []);
    let answer: (line: Line) => void = () => {};
    const adding = pushAfter<Line[]>(queryClient, key, async () =>
      add(await new Promise<Line>((resolve) => (answer = resolve))),
    );
    // `commentAdded`, then the comment is removed while it is being fetched.
    pushTo<Line[]>(queryClient, key, remove("c1"));
    answer({ id: "c1", version: 1 });
    await adding;
    expect(queryClient.getQueryData(key)).toEqual([]);
  });

  it("reaches the query's own read in flight, in the order things happened", async () => {
    const queryClient = new QueryClient();
    const api = server();
    queryClient.setQueryData<Line[]>(key, []);
    const loading = queryClient.fetchQuery({ ...api.read, queryKey: key, staleTime: 0 });
    await pushAfter<Line[]>(queryClient, key, async () => add({ id: "mine", version: 1 }));
    expect(queryClient.getQueryData(key)).toEqual([{ id: "mine", version: 1 }]);
    // The list was read before the answer that added it.
    api.answer(0, [{ id: "old", version: 1 }]);
    await loading;
    expect(queryClient.getQueryData(key)).toEqual([
      { id: "old", version: 1 },
      { id: "mine", version: 1 },
    ]);
  });

  it("changes nothing when the read has nothing to add, or fails", async () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData<Line[]>(key, [{ id: "a", version: 1 }]);
    await pushAfter<Line[]>(queryClient, key, async () => undefined);
    await expect(
      pushAfter<Line[]>(queryClient, key, async () => {
        throw new Error("gone");
      }),
    ).rejects.toThrow("gone");
    pushTo<Line[]>(queryClient, key, add({ id: "b", version: 1 }));
    expect(queryClient.getQueryData(key)).toEqual([
      { id: "a", version: 1 },
      { id: "b", version: 1 },
    ]);
  });
});
