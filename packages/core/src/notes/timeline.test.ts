import { create, type MessageInitShape, notes } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { bumpCounter, mapNotes, withToggle } from "./cache";
import {
  countNewEntries,
  entryKey,
  type TimelineData,
  type TimelinePage,
  withFreshEntries,
} from "./timeline";

const note = (id: string, extra: MessageInitShape<typeof notes.NoteSchema> = {}) =>
  create(notes.NoteSchema, { id, text: id, ...extra });
const entry = (id: string) => create(notes.TimelineEntrySchema, { note: note(id) });
const page = (ids: string[], nextCursor = ""): TimelinePage => ({
  entries: ids.map(entry),
  nextCursor,
});
const data = (...pages: TimelinePage[]): TimelineData => ({
  pages,
  pageParams: pages.map(() => ""),
});
const ids = (timeline: TimelineData) =>
  timeline.pages.map((item) => item.entries.map((row) => row.note?.id));

describe("new entries of a timeline", () => {
  it("counts the rows above the first one on screen", () => {
    expect(countNewEntries(data(page(["c", "b", "a"])), page(["e", "d", "c", "b"]))).toBe(2);
    expect(countNewEntries(data(page(["c", "b"])), page(["c", "b"]))).toBe(0);
  });

  it("cannot say how many when the fresh page does not reach the rows on screen", () => {
    expect(countNewEntries(data(page(["a"])), page(["z", "y", "x"]))).toBeUndefined();
  });

  it("treats an empty timeline as all new", () => {
    expect(countNewEntries(undefined, page(["b", "a"]))).toBe(2);
    expect(countNewEntries(data(page([])), page(["a"]))).toBe(1);
  });

  it("puts new rows on top and keeps the pages below", () => {
    const merged = withFreshEntries(
      data(page(["c", "b"], "more"), page(["a"])),
      page(["e", "d", "c"]),
    );
    expect(ids(merged)).toEqual([["e", "d", "c", "b"], ["a"]]);
    expect(merged.pages[0]?.nextCursor).toBe("more");
  });

  it("moves a note that came back to the top out of its old place", () => {
    // "a" was renoted again: it is new on top and must not show twice.
    const renoted = create(notes.TimelineEntrySchema, {
      note: note("a"),
      renote: { user: { id: "u1" }, renotedAt: { seconds: 10n, nanos: 0 } },
    });
    const merged = withFreshEntries(data(page(["c", "b"]), page(["a"])), {
      entries: [renoted, entry("c")],
      nextCursor: "",
    });
    expect(ids(merged)).toEqual([["a", "c", "b"], []]);
  });

  it("starts over from the fresh page when there is a gap", () => {
    expect(ids(withFreshEntries(data(page(["a"])), page(["z", "y"])))).toEqual([["z", "y"]]);
  });

  it("tells a renote row apart from the note's own row", () => {
    const plain = entry("a");
    const renoted = create(notes.TimelineEntrySchema, {
      note: note("a"),
      renote: { user: { id: "u1" }, renotedAt: { seconds: 10n, nanos: 0 } },
    });
    expect(entryKey(plain)).not.toBe(entryKey(renoted));
  });
});

describe("mapNotes", () => {
  it("updates a note wherever it sits, quoted notes included, and keeps the rest as it was", () => {
    const quoted = note("q");
    const quoting = note("x", {
      quote: create(notes.QuotedNoteSchema, { noteId: "q", note: quoted }),
    });
    const untouched = entry("other");
    const timeline = data({
      entries: [create(notes.TimelineEntrySchema, { note: quoting }), untouched],
      nextCursor: "",
    });

    const next = mapNotes(timeline, (item) =>
      item.id === "q" ? { ...item, text: "changed" } : item,
    );
    expect(next.pages[0]?.entries[0]?.note?.quote?.note?.text).toBe("changed");
    expect(next.pages[0]?.entries[1]).toBe(untouched);
    expect(mapNotes(timeline, (item) => item)).toBe(timeline);
  });
});

describe("bumpCounter", () => {
  it("moves one counter and never goes below zero", () => {
    const fresh = note("a");
    expect(bumpCounter(fresh, "likeCount", 1).likeCount).toBe(1n);
    expect(bumpCounter(fresh, "replyCount", -1).replyCount).toBe(0n);
    expect(bumpCounter(fresh, "likeCount", 1).noteId).toBe("a");
  });
});

describe("withToggle", () => {
  it("moves the mark and the count together, once", () => {
    const liked = withToggle(note("a"), "like", true);
    expect(liked.viewer?.liked).toBe(true);
    expect(liked.counters?.likeCount).toBe(1n);
    expect(withToggle(liked, "like", true)).toBe(liked);
    const undone = withToggle(liked, "like", false);
    expect(undone.viewer?.liked).toBe(false);
    expect(undone.counters?.likeCount).toBe(0n);
    expect(withToggle(note("b"), "renote", true).counters?.renoteCount).toBe(1n);
  });
});
