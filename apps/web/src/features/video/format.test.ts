import { describe, expect, it } from "vitest";
import { formatAge, formatCount, isoDay, toLocalInput } from "./format";

// Clocks and IDs are tested with `@tokuly/core/video`; these are the web's own and its bindings.
describe("video formatting", () => {
  it("speaks the web's language", () => {
    expect(formatCount(12_345)).toBe("1.2万");
    const now = 1_800_000_000_000;
    expect(formatAge(now - 3 * 3_600_000, now)).toBe("3時間前");
  });

  it("formats local days and datetime-local values", () => {
    const date = new Date(2026, 2, 5, 9, 7);
    expect(isoDay(date)).toBe("2026-03-05");
    expect(toLocalInput(date)).toBe("2026-03-05T09:07");
  });
});
