import { describe, expect, it } from "vitest";
import { pushesDuringRead, pushesDuringReadByKey } from "./pushes-during-read";

const add = (item: string) => (state: string[]) =>
  state.includes(item) ? state : [...state, item];
const remove = (item: string) => (state: string[]) => state.filter((other) => other !== item);

describe("pushes during a read", () => {
  it("applies what was pushed while the read was in flight to its answer, in order", () => {
    const journal = pushesDuringRead<string[]>();
    journal.record(add("before"));
    const read = journal.begin();
    journal.record(add("during"));
    journal.record(remove("old"));
    // The answer was made before both pushes.
    expect(read.finish(["old", "before"])).toEqual(["before", "during"]);
    // Once read, nothing more is kept for it.
    journal.record(add("after"));
    expect(read.finish(["x"])).toEqual(["x", "during"]);
  });

  it("keeps nothing for a read that was abandoned, and separates keys", () => {
    const journal = pushesDuringReadByKey<string[]>();
    const abandoned = journal.begin("a");
    abandoned.cancel();
    abandoned.cancel();
    journal.record("a", add("lost"));
    const a = journal.begin("a");
    const b = journal.begin("b");
    journal.record("a", add("for a"));
    expect(b.finish([])).toEqual([]);
    expect(a.finish([])).toEqual(["for a"]);
  });
});
