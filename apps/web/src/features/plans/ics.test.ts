import { describe, expect, it } from "vitest";
import { buildIcs } from "./ics";

const NOW = new Date(Date.UTC(2026, 10, 22, 3, 0, 0));

describe("buildIcs", () => {
  it("writes a timed event in UTC, an hour long when it has no end", () => {
    const text = buildIcs([{ uid: "plan-1", title: "カラオケ", start: new Date(Date.UTC(2026, 11, 24, 9, 30)) }], NOW);
    expect(text).toContain("DTSTART:20261224T093000Z");
    expect(text).toContain("DTEND:20261224T103000Z");
    expect(text).toContain("UID:plan-1@tokuly");
    expect(text).toContain("SUMMARY:カラオケ");
    expect(text.startsWith("BEGIN:VCALENDAR\r\n")).toBe(true);
    expect(text.endsWith("END:VCALENDAR\r\n")).toBe(true);
  });

  it("writes a day without a time as an all-day event ending the next day", () => {
    const text = buildIcs([{ uid: "c-1", title: "文化祭", date: "2026-12-31" }], NOW);
    expect(text).toContain("DTSTART;VALUE=DATE:20261231");
    expect(text).toContain("DTEND;VALUE=DATE:20270101");
  });

  it("escapes punctuation and line breaks in text", () => {
    const text = buildIcs([{ uid: "p", title: "ごはん, そのあと; 映画", date: "2026-12-01", description: "一行目\n二行目" }], NOW);
    expect(text).toContain("SUMMARY:ごはん\\, そのあと\\; 映画");
    expect(text).toContain("DESCRIPTION:一行目\\n二行目");
  });

  it("folds long lines to 75 bytes", () => {
    const text = buildIcs([{ uid: "p", title: "あ".repeat(60), date: "2026-12-01" }], NOW);
    for (const line of text.split("\r\n")) expect(new TextEncoder().encode(line).length).toBeLessThanOrEqual(75);
    expect(text.replace(/\r\n /g, "")).toContain(`SUMMARY:${"あ".repeat(60)}`);
  });

  it("leaves out entries without a day", () => {
    expect(buildIcs([{ uid: "p", title: "未定" }], NOW)).not.toContain("VEVENT");
  });
});
