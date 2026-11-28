import type { Client } from "@connectrpc/connect";
import { QueryClient } from "@tanstack/query-core";
import { create, notes } from "@tokuly/api";
import { describe, expect, it, vi } from "vitest";
import { noteLoader, readNoteAgain } from "./batch";
import { notesKeys } from "./keys";
import type { TimelineData } from "./timeline";

const note = (id: string, text: string, editedSeconds = 0n) =>
  create(notes.NoteSchema, {
    id,
    text,
    editedAt: editedSeconds ? { seconds: editedSeconds } : undefined,
    counters: { noteId: id, likeCount: 1n },
  });

/** A server that has `held` and answers every `BatchGetNotes` with those it is asked for. */
function server(held: notes.Note[]) {
  const asked: string[][] = [];
  const client = () =>
    ({
      batchGetNotes: async ({ noteIds }: { noteIds: string[] }) => {
        asked.push(noteIds);
        return { notes: held.filter((item) => noteIds.includes(item.id)) };
      },
    }) as unknown as Client<typeof notes.NoteService>;
  return { load: noteLoader(client), asked };
}

function onScreen(...shown: notes.Note[]) {
  const queryClient = new QueryClient();
  queryClient.setQueryData<TimelineData>(notesKeys.timeline({ kind: "home" }), {
    pages: [
      {
        entries: shown.map((item) => create(notes.TimelineEntrySchema, { note: item })),
        nextCursor: "",
      },
    ],
    pageParams: [""],
  });
  return queryClient;
}

const texts = (queryClient: QueryClient) =>
  queryClient
    .getQueryData<TimelineData>(notesKeys.timeline({ kind: "home" }))
    ?.pages[0]?.entries.map((entry) => entry.note?.text);

describe("reading a card's note again once its topic is in effect", () => {
  it("shows the words of an edit made before the topic took effect", async () => {
    const queryClient = onScreen(note("n", "前の言葉"));
    const api = server([note("n", "直した言葉", 5n)]);
    await expect(readNoteAgain(queryClient, api.load, "n")).resolves.toBe(false);
    expect(texts(queryClient)).toEqual(["直した言葉"]);
  });

  it("says a note deleted before the topic took effect is gone", async () => {
    const queryClient = onScreen(note("n", "消された"));
    const api = server([]);
    await expect(readNoteAgain(queryClient, api.load, "n")).resolves.toBe(true);
  });

  it("reads the cards that appear together in one request", async () => {
    const api = server([note("a", "a"), note("b", "b")]);
    const [a, b] = await Promise.all([api.load("a"), api.load("b"), api.load("a")]);
    expect([a?.id, b?.id]).toEqual(["a", "b"]);
    expect(api.asked).toEqual([["a", "b"]]);
  });

  it("replaces a read of the note's own query that began before", async () => {
    const queryClient = new QueryClient();
    const api = server([note("n", "新しい")]);
    let answer: (value: notes.Note) => void = () => {};
    const older = vi.fn(() => new Promise<notes.Note>((resolve) => (answer = resolve)));
    const first = queryClient.fetchQuery({ queryKey: notesKeys.note("n"), queryFn: older });
    queryClient
      .getQueryCache()
      .find({ queryKey: notesKeys.note("n") })
      ?.setOptions({
        queryKey: notesKeys.note("n"),
        queryFn: () => api.load("n"),
      });
    await readNoteAgain(queryClient, api.load, "n");
    answer(note("n", "古い"));
    await first.catch(() => {});
    await vi.waitFor(() =>
      expect(queryClient.getQueryData<notes.Note>(notesKeys.note("n"))?.text).toBe("新しい"),
    );
  });
});
