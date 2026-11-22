import { school } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { collides, entryFor, type GridEntry, slotKey, slots } from "./grid-model";

const { EVERY, A, B } = school.WeekPattern;
const MON = school.Weekday.MONDAY;

function entry(
  id: string,
  weekday: school.Weekday,
  period: number,
  weekPattern: school.WeekPattern,
): GridEntry {
  return {
    id,
    weekday,
    period,
    weekPattern,
    subject: undefined,
    room: "",
    teacher: "",
    color: "",
    memo: "",
  };
}

describe("slots", () => {
  it("groups a week's classes by slot, keeping A and B weeks apart", () => {
    const map = slots([entry("e", MON, 1, EVERY), entry("a", MON, 2, A), entry("b", MON, 2, B)]);
    expect(map.get(slotKey(MON, 1))?.every?.id).toBe("e");
    expect(map.get(slotKey(MON, 2))).toMatchObject({ a: { id: "a" }, b: { id: "b" } });
    expect(map.get(slotKey(MON, 3))).toBeUndefined();
  });

  it("finds the class of a slot in a given week", () => {
    const slot = { a: entry("a", MON, 2, A), every: undefined, b: undefined };
    expect(entryFor(slot, A)?.id).toBe("a");
    expect(entryFor(slot, B)).toBeUndefined();
    expect(entryFor({ every: entry("e", MON, 1, EVERY) }, B)?.id).toBe("e");
  });
});

describe("collides", () => {
  it("lets an every-week class replace everything in its slot", () => {
    expect(collides(entry("a", MON, 1, A), MON, 1, EVERY)).toBe(true);
    expect(collides(entry("e", MON, 1, EVERY), MON, 1, EVERY)).toBe(true);
  });

  it("lets a week-A class replace the every-week and the week-A class, not week B's", () => {
    expect(collides(entry("e", MON, 1, EVERY), MON, 1, A)).toBe(true);
    expect(collides(entry("a", MON, 1, A), MON, 1, A)).toBe(true);
    expect(collides(entry("b", MON, 1, B), MON, 1, A)).toBe(false);
  });

  it("never touches another slot", () => {
    expect(collides(entry("e", MON, 2, EVERY), MON, 1, EVERY)).toBe(false);
    expect(collides(entry("e", school.Weekday.TUESDAY, 1, EVERY), MON, 1, EVERY)).toBe(false);
  });
});
