import { create, type MessageInitShape, quest } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  badgeLines,
  claimable,
  formatDay,
  formatShare,
  fraction,
  loudness,
  nextInLine,
  remaining,
  streakRuns,
  streakStatus,
} from "./model";

const stamp = (iso: string) => {
  const ms = Date.parse(iso);
  return { seconds: BigInt(Math.floor(ms / 1000)), nanos: 0 };
};

const badge = (init: MessageInitShape<typeof quest.BadgeSchema>) => create(quest.BadgeSchema, init);
const mission = (id: string, state: quest.MissionState) =>
  create(quest.MissionSchema, { id, state });

describe("fraction", () => {
  it("stays between 0 and 1 and survives an empty target", () => {
    expect(fraction(5n, 10n)).toBe(0.5);
    expect(fraction(15n, 10n)).toBe(1);
    expect(fraction(3n, 0n)).toBe(0);
  });
});

describe("formatShare", () => {
  it("keeps small shares readable", () => {
    expect(formatShare(0.42)).toBe("42%");
    expect(formatShare(0.032)).toBe("3.2%");
    expect(formatShare(0.0004)).toBe("<0.1%");
  });
});

describe("badgeLines", () => {
  it("puts the tiers of a line together, lowest first, and leaves single badges alone", () => {
    const lines = badgeLines([
      badge({ key: "chatter_gold", line: "chatter", tier: quest.BadgeTier.GOLD }),
      badge({ key: "chatter_bronze", line: "chatter", tier: quest.BadgeTier.BRONZE }),
      badge({ key: "welcome", line: "welcome" }),
      badge({ key: "secret_1", secret: true }),
      badge({ key: "secret_2", secret: true }),
    ]);
    expect(lines.map((line) => line.map((item) => item.key))).toEqual([
      ["chatter_bronze", "chatter_gold"],
      ["welcome"],
      ["secret_1"],
      ["secret_2"],
    ]);
  });

  it("names the tier to work towards", () => {
    const line = [
      badge({ key: "a_bronze", owned: true }),
      badge({ key: "a_silver" }),
      badge({ key: "a_gold" }),
    ];
    expect(nextInLine(line)?.key).toBe("a_silver");
    expect(nextInLine(line.map((item) => ({ ...item, owned: true })))?.key).toBe("a_gold");
  });
});

describe("streakStatus", () => {
  const zone = "Asia/Tokyo";
  // 2026-11-23 10:00 in Tokyo.
  const now = Date.parse("2026-11-23T01:00:00Z");
  const streak = (init: MessageInitShape<typeof quest.StreakSchema>) =>
    create(quest.StreakSchema, { days: 3, state: quest.StreakState.ACTIVE, ...init });

  it("says whose turn it is while today is open", () => {
    // Ends at tonight's midnight in Tokyo.
    const tonight = stamp("2026-11-23T15:00:00Z");
    expect(
      streakStatus(streak({ waitingOn: quest.StreakTurn.MINE, expiresAt: tonight }), now, zone),
    ).toBe("mine");
    expect(
      streakStatus(streak({ waitingOn: quest.StreakTurn.THEIRS, expiresAt: tonight }), now, zone),
    ).toBe("theirs");
  });

  it("is done for today once the open day is tomorrow", () => {
    const tomorrowNight = stamp("2026-11-24T15:00:00Z");
    expect(
      streakStatus(
        streak({ waitingOn: quest.StreakTurn.BOTH, expiresAt: tomorrowNight }),
        now,
        zone,
      ),
    ).toBe("done");
  });

  it("follows the state for streaks that are about to end, ended or never started", () => {
    expect(streakStatus(streak({ state: quest.StreakState.EXPIRING }), now, zone)).toBe("expiring");
    expect(streakStatus(streak({ state: quest.StreakState.LOST }), now, zone)).toBe("lost");
    expect(streakStatus(streak({ state: quest.StreakState.NONE, days: 0 }), now, zone)).toBe(
      "none",
    );
  });

  it("only calls a running streak running", () => {
    expect(streakRuns(streak({}))).toBe(true);
    expect(streakRuns(streak({ state: quest.StreakState.LOST }))).toBe(false);
    expect(streakRuns(streak({ days: 0 }))).toBe(false);
    expect(streakRuns(undefined)).toBe(false);
  });
});

describe("remaining", () => {
  const now = 1_000_000_000_000;
  it("uses the unit a countdown is read in", () => {
    expect(remaining(now + 3 * 24 * 3_600_000, now)).toEqual({ unit: "days", count: 3 });
    expect(remaining(now + 30 * 3_600_000, now)).toEqual({ unit: "hours", count: 30 });
    expect(remaining(now + 12 * 60_000 - 1, now)).toEqual({ unit: "minutes", count: 12 });
    expect(remaining(now - 5, now)).toEqual({ unit: "minutes", count: 0 });
  });
});

describe("formatDay", () => {
  it("reads the date as written, whatever zone the device is in", () => {
    expect(formatDay("2026-11-23", "en")).toBe("Nov 23");
    expect(formatDay("2026-01-01", "ja", true)).toBe("2026年1月1日");
  });
});

describe("claimable", () => {
  it("collects completed missions once each", () => {
    const done = mission("b", quest.MissionState.COMPLETED);
    const list = create(quest.ListMissionsResponseSchema, {
      daily: [mission("a", quest.MissionState.ACTIVE), done],
      weekly: mission("w", quest.MissionState.CLAIMED),
      unclaimed: [mission("old", quest.MissionState.COMPLETED), done],
    });
    expect(claimable(list).map((item) => item.id)).toEqual(["b", "old"]);
    expect(claimable(undefined)).toEqual([]);
  });
});

describe("loudness", () => {
  it("grows with the ドパガキ度", () => {
    expect(loudness(0)).toBe("toast");
    expect(loudness(0.5)).toBe("toast");
    expect(loudness(1)).toBe("card");
    expect(loudness(1.6)).toBe("full");
  });
});
