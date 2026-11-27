import { create } from "@bufbuild/protobuf";
import { identity, quest } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { profileQuest } from "./profile";

const profile = (cards: { kind: string; data: Record<string, string> }[]) =>
  create(identity.ProfileSchema, { cards });

describe("profileQuest", () => {
  it("reads the level, the title, the showcase and the streak", () => {
    const read = profileQuest(
      profile([
        {
          kind: "quest.level",
          data: {
            level: "12",
            title_ja: "旅人",
            title_en: "Traveller",
            badges: JSON.stringify([
              { key: "chatter_gold", emoji: "💬", tier: 3, name_ja: "おしゃべり" },
              { nope: 1 },
            ]),
          },
        },
        { kind: "quest.streak", data: { days: "7", state: String(quest.StreakState.EXPIRING) } },
      ]),
    );
    expect(read).toMatchObject({
      shown: true,
      level: 12,
      title: { ja: "旅人", en: "Traveller" },
      streakDays: 7,
      streakExpiring: true,
    });
    expect(
      read.showcase.map((badge) => [badge.key, badge.tier, badge.name?.ja, badge.owned]),
    ).toEqual([["chatter_gold", quest.BadgeTier.GOLD, "おしゃべり", true]]);
  });

  it("shows nothing for someone who keeps their progress to themselves, and survives a broken showcase", () => {
    expect(profileQuest(profile([]))).toMatchObject({
      shown: false,
      level: 0,
      showcase: [],
      streakDays: 0,
    });
    expect(
      profileQuest(profile([{ kind: "quest.level", data: { badges: "{" } }])).showcase,
    ).toEqual([]);
  });
});
