import { create } from "@bufbuild/protobuf";
import { QueryClient } from "@tanstack/query-core";
import { quest } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { applyQuestEvent, showClaim, showReroll } from "./cache";
import { questKeys } from "./keys";

const mission = (id: string, state = quest.MissionState.ACTIVE) =>
  create(quest.MissionSchema, { id, state });

function withMissions() {
  const queryClient = new QueryClient();
  queryClient.setQueryData(
    questKeys.missions,
    create(quest.ListMissionsResponseSchema, {
      daily: [mission("d1", quest.MissionState.COMPLETED), mission("d2")],
      weekly: mission("w1"),
      unclaimed: [mission("old", quest.MissionState.COMPLETED)],
      rerollsLeft: 1,
    }),
  );
  const missions = () =>
    queryClient.getQueryData<quest.ListMissionsResponse>(
      questKeys.missions,
    ) as quest.ListMissionsResponse;
  return { queryClient, missions };
}

describe("mission cache edits", () => {
  it("a claim marks the mission claimed, drops it from the unclaimed ones and keeps the bonus and level", () => {
    const { queryClient, missions } = withMissions();
    const progress = create(quest.ProgressSchema, { xpToday: 40 });
    showClaim(
      queryClient,
      create(quest.ClaimMissionResponseSchema, {
        mission: mission("old", quest.MissionState.CLAIMED),
        dailyBonus: { claimed: 1, required: 3 },
        progress,
        granted: true,
      }),
      mission("old"),
    );
    expect(missions().unclaimed).toEqual([]);
    expect(missions().dailyBonus?.claimed).toBe(1);
    expect(queryClient.getQueryData(questKeys.progress)).toEqual(progress);

    showClaim(
      queryClient,
      create(quest.ClaimMissionResponseSchema, {
        mission: mission("d1", quest.MissionState.CLAIMED),
      }),
      mission("d1"),
    );
    expect(missions().daily[0]?.state).toBe(quest.MissionState.CLAIMED);
    expect(missions().dailyBonus?.claimed).toBe(1);
  });

  it("a reroll puts the new mission in the slot and counts the reroll", () => {
    const { queryClient, missions } = withMissions();
    const next = create(quest.MissionSchema, { id: "w1", key: "other" });
    showReroll(
      queryClient,
      create(quest.RerollMissionResponseSchema, { mission: next, rerollsLeft: 0 }),
    );
    expect(missions().weekly?.key).toBe("other");
    expect(missions().rerollsLeft).toBe(0);
  });

  it("a completed mission pushed by the server shows at once", () => {
    const { queryClient, missions } = withMissions();
    applyQuestEvent(
      queryClient,
      create(quest.QuestEventSchema, {
        event: { case: "missionCompleted", value: mission("d2", quest.MissionState.COMPLETED) },
      }),
    );
    expect(missions().daily[1]?.state).toBe(quest.MissionState.COMPLETED);
  });
});
