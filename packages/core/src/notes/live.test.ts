import { QueryClient } from "@tanstack/query-core";
import { create, notes } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { notesKeys } from "./keys";
import { applyNotesEvent } from "./live";
import type { TimelineData } from "./timeline";

function cached(replyCount: bigint) {
  const queryClient = new QueryClient();
  const note = create(notes.NoteSchema, { id: "n", counters: { noteId: "n", replyCount } });
  queryClient.setQueryData<TimelineData>(notesKeys.timeline({ kind: "home" }), {
    pages: [{ entries: [create(notes.TimelineEntrySchema, { note })], nextCursor: "" }],
    pageParams: [""],
  });
  return queryClient;
}

const counters = (replyCount: bigint): notes.NotesEvent["event"] => ({
  case: "countersChanged",
  value: create(notes.NoteCountersSchema, { noteId: "n", replyCount }),
});

describe("applyNotesEvent", () => {
  it("puts pushed counters into the cache and hints when a note on screen got a reply", () => {
    const queryClient = cached(1n);
    expect(applyNotesEvent(queryClient, counters(2n))).toEqual({ hint: true });
    const data = queryClient.getQueryData<TimelineData>(notesKeys.timeline({ kind: "home" }));
    expect(data?.pages[0]?.entries[0]?.note?.counters?.replyCount).toBe(2n);
    expect(applyNotesEvent(queryClient, counters(2n))).toEqual({});
  });

  it("leaves deletions and edits to the app", () => {
    const queryClient = new QueryClient();
    expect(applyNotesEvent(queryClient, { case: "noteDeletedId", value: "n" })).toEqual({
      deleted: "n",
    });
    expect(
      applyNotesEvent(queryClient, {
        case: "noteEdited",
        value: create(notes.NoteEditedSchema, { noteId: "n" }),
      }),
    ).toEqual({ refresh: "n" });
  });
});
