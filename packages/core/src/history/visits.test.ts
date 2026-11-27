import { create, fromDate, geo } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { testI18n } from "../testing/i18n";
import { visitEmoji, visitName, visitTimes } from "./visits";

const ja = testI18n("ja");
const at = (iso: string) => fromDate(new Date(iso));

describe("visits", () => {
  it("are named and drawn after their place, else the spot", () => {
    const home = create(geo.PlaceSchema, { name: "家", label: geo.PlaceLabel.HOME });
    expect(visitName(create(geo.VisitSchema, { place: home, name: "x" }))).toBe("家");
    expect(visitEmoji(create(geo.VisitSchema, { place: home }))).toBe("🏠");
    expect(visitName(create(geo.VisitSchema, { locality: "渋谷区" }))).toBe("渋谷区");
    expect(visitEmoji(create(geo.VisitSchema, { checkIn: true }))).toBe("✅");
  });

  it("tell their times on the account's clock, whatever the device's", () => {
    const stay = create(geo.VisitSchema, {
      arrivedAt: at("2026-11-22T08:16:00Z"),
      departedAt: at("2026-11-22T08:25:00Z"),
      durationSeconds: 540n,
    });
    expect(visitTimes(ja, stay, "Asia/Tokyo")).toBe("17:16–17:25・9分");
    expect(visitTimes(ja, stay, "UTC")).toBe("08:16–08:25・9分");
    const ongoing = create(geo.VisitSchema, { arrivedAt: at("2026-11-22T08:33:00Z") });
    expect(visitTimes(ja, ongoing, "Asia/Tokyo")).toBe(
      ja.t("history:visit.ongoing", { time: "17:33" }),
    );
  });

  it("say the day a stay began on when it ended on another", () => {
    const night = create(geo.VisitSchema, {
      arrivedAt: at("2026-11-21T13:00:00Z"),
      departedAt: at("2026-11-21T23:00:00Z"),
      durationSeconds: 36_000n,
    });
    expect(visitTimes(ja, night, "Asia/Tokyo")).toBe("11/21 22:00–08:00・10時間");
  });
});
