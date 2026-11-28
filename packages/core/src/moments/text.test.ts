import { describe, expect, it } from "vitest";
import { testI18n } from "../testing/i18n";
import { countdownText, latenessText } from "./text";

describe("latenessText", () => {
  const ja = testI18n("ja");

  it("says on time, or how late in minutes and then hours", () => {
    expect(latenessText(ja, 0n)).toBe("時間内!");
    expect(latenessText(ja, 1)).toBe("1分遅れ");
    expect(latenessText(ja, 59 * 60)).toBe("59分遅れ");
    expect(latenessText(ja, 3 * 3600 + 120)).toBe("3時間遅れ");
    expect(latenessText(testI18n("en"), 3 * 3600)).toBe("3 h late");
  });
});

describe("countdownText", () => {
  it("rounds the second up and stops at zero", () => {
    expect(countdownText(120_000)).toBe("2:00");
    expect(countdownText(59_001)).toBe("1:00");
    expect(countdownText(9_000)).toBe("0:09");
    expect(countdownText(-5)).toBe("0:00");
  });
});
