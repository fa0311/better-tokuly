import { InfiniteQueryObserver, QueryClient } from "@tanstack/query-core";
import { create, notes } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { forgetUserNotes } from "./cache";
import { notesKeys, type TimelineSource } from "./keys";
import { putRefreshedNote } from "./live";
import {
  lookForNewEntries,
  type TimelineClients,
  type TimelineData,
  type TimelinePage,
  timelineQuery,
} from "./timeline";

const home: TimelineSource = { kind: "home" };
const key = notesKeys.timeline(home);

const entry = (id: string, authorId: string) =>
  create(notes.TimelineEntrySchema, {
    note: create(notes.NoteSchema, { id, author: { id: authorId } }),
  });

/** A server whose home timeline pages the test hands out, one per read. */
function server() {
  const pending: ((response: notes.GetHomeTimelineResponse) => void)[] = [];
  const clients = {
    notes: () => ({
      getHomeTimeline: () =>
        new Promise<notes.GetHomeTimelineResponse>((resolve) => pending.push(resolve)),
    }),
  } as unknown as TimelineClients;
  return {
    clients,
    answer(read: number, entries: notes.TimelineEntry[], nextCursor = "") {
      pending[read]?.(
        create(notes.GetHomeTimelineResponseSchema, { entries, page: { nextCursor } }),
      );
    },
  };
}

/** The ids of the notes in the cached home timeline, page by page. */
const shown = (queryClient: QueryClient) =>
  queryClient
    .getQueryData<TimelineData>(key)
    ?.pages.map((page) => page.entries.map((item) => item.note?.id));

/** A home timeline on screen, its first page read: `[mine, theirs]`. */
async function onScreen() {
  const queryClient = new QueryClient();
  const api = server();
  const observer = new InfiniteQueryObserver(queryClient, timelineQuery(api.clients, home));
  const unsubscribe = observer.subscribe(() => {});
  api.answer(0, [entry("mine", "me"), entry("theirs", "ex")], "next");
  await observer.refetch();
  return { queryClient, api, observer, unsubscribe };
}

describe("forgetting a person in the timelines", () => {
  it("keeps them out of a page that was being read when they were forgotten", async () => {
    const { queryClient, api, observer, unsubscribe } = await onScreen();
    const next = observer.fetchNextPage();
    forgetUserNotes(queryClient, "ex");
    expect(shown(queryClient)).toEqual([["mine"]]);
    // The second page was read before the unfriend.
    api.answer(1, [entry("older", "ex"), entry("oldest", "me")]);
    await next;
    expect(shown(queryClient)).toEqual([["mine"], ["oldest"]]);
    unsubscribe();
  });

  it("keeps them out of every page read again", async () => {
    const { queryClient, api, observer, unsubscribe } = await onScreen();
    const again = observer.refetch();
    forgetUserNotes(queryClient, "ex");
    api.answer(1, [entry("mine", "me"), entry("theirs", "ex")]);
    await again;
    expect(shown(queryClient)).toEqual([["mine"]]);
    unsubscribe();
  });

  it("keeps them out of the newest page a look read before", async () => {
    const { queryClient, unsubscribe } = await onScreen();
    let answer: (page: TimelinePage) => void = () => {};
    const look = lookForNewEntries(
      queryClient,
      home,
      () => new Promise((resolve) => (answer = resolve)),
      () => true,
    );
    forgetUserNotes(queryClient, "ex");
    answer({ entries: [entry("new", "ex"), entry("mine", "me")], nextCursor: "" });
    await look;
    expect(shown(queryClient)).toEqual([["mine"]]);
    unsubscribe();
  });

  it("keeps them out of new notes waiting behind the pill", async () => {
    const { queryClient, unsubscribe } = await onScreen();
    let tap: (yes: boolean) => void = () => {};
    const look = lookForNewEntries(
      queryClient,
      home,
      async () => ({
        entries: [entry("new", "ex"), entry("newer", "me"), entry("mine", "me")],
        nextCursor: "",
      }),
      () => new Promise<boolean>((resolve) => (tap = resolve)),
    );
    await new Promise((resolve) => setTimeout(resolve));
    // Unfriended while the pill said "2 new notes"; then the pill is tapped.
    forgetUserNotes(queryClient, "ex");
    tap(true);
    await look;
    expect(shown(queryClient)).toEqual([["newer", "mine"]]);
    unsubscribe();
  });

  it("leaves the timeline as it was when the pill is dropped", async () => {
    const { queryClient, unsubscribe } = await onScreen();
    const look = lookForNewEntries(
      queryClient,
      home,
      async () => ({ entries: [entry("new", "me"), entry("mine", "me")], nextCursor: "" }),
      () => false,
    );
    await look;
    expect(shown(queryClient)).toEqual([["mine", "theirs"]]);
    unsubscribe();
  });
});

describe("a note read again", () => {
  it("does not go back to the words of an earlier edit", () => {
    const queryClient = new QueryClient();
    const note = (text: string, seconds: bigint) =>
      create(notes.NoteSchema, { id: "n", text, editedAt: { seconds } });
    queryClient.setQueryData<TimelineData>(key, {
      pages: [
        { entries: [create(notes.TimelineEntrySchema, { note: note("v1", 1n) })], nextCursor: "" },
      ],
      pageParams: [""],
    });
    putRefreshedNote(queryClient, note("v3", 3n));
    // The answer to the read after the second edit lands last.
    putRefreshedNote(queryClient, note("v2", 2n));
    const data = queryClient.getQueryData<TimelineData>(key);
    expect(data?.pages[0]?.entries[0]?.note?.text).toBe("v3");
  });
});
