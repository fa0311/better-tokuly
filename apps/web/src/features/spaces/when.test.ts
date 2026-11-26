import { describe, expect, it } from "vitest";
import { clockOf, minuteOf, suggestedStart } from "./when";

describe("schedule times", () => {
  it("reads and writes the time input", () => {
    expect(clockOf(18 * 60 + 5)).toBe("18:05");
    expect(minuteOf("09:30")).toBe(570);
    expect(minuteOf("24:00")).toBeNull();
    expect(minuteOf("nine")).toBeNull();
  });

  it("proposes the next full hour at least half an hour away, on the account's clock", () => {
    // 2026-11-23 10:40 in Tokyo is 01:40 UTC.
    const now = Date.UTC(2026, 10, 23, 1, 40);
    expect(suggestedStart(now, "Asia/Tokyo")).toEqual({ date: "2026-11-23", clock: "12:00" });
    // The same moment is the day before in Pago Pago.
    expect(suggestedStart(now, "Pacific/Pago_Pago")).toEqual({
      date: "2026-11-22",
      clock: "16:00",
    });
  });
});
