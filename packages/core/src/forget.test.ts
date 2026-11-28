import { QueryClient, type QueryKey, QueryObserver } from "@tanstack/query-core";
import { describe, expect, it } from "vitest";
import { forgetUserAsk } from "./ask/forget";
import { askKeys } from "./ask/keys";
import { forgetUserFootprints } from "./footprints/forget";
import { footprintKeys } from "./footprints/keys";
import { forgetUserHistory } from "./history/forget";
import { historyKeys } from "./history/keys";
import { forgetUserLive } from "./live/forget";
import { liveKeys } from "./live/keys";
import { forgetUserMap } from "./map/forget";
import { mapKeys } from "./map/keys";
import { forgetUserPlaces } from "./places/forget";
import { placeKeys } from "./places/keys";
import { forgetUserPlans } from "./plans/forget";
import { planKeys } from "./plans/keys";
import { forgetUserProfile } from "./profile/forget";
import { profileKeys } from "./profile/keys";
import { forgetUserQuest } from "./quest/forget";
import { questKeys } from "./quest/keys";
import { forgetUserSchool } from "./school/forget";
import { schoolKeys } from "./school/keys";
import { forgetUserSpaces } from "./spaces/forget";
import { spacesKeys } from "./spaces/keys";
import { forgetUserVideo } from "./video/forget";
import { videoKeys } from "./video/keys";

type Forget = (queryClient: QueryClient, userId: string) => Promise<void>;

/** Each area's forgetter, with a query keyed by the person ("ex") and a list of friends' content. */
const AREAS: [string, Forget, QueryKey[]][] = [
  ["ask", forgetUserAsk, [askKeys.box("ex"), askKeys.friendPrompts]],
  [
    "footprints",
    forgetUserFootprints,
    [footprintKeys.stats("ex"), footprintKeys.leaderboard("world", 1)],
  ],
  ["history", forgetUserHistory, [historyKeys.day("2026-11-29", "Asia/Tokyo")]],
  ["live", forgetUserLive, [liveKeys.friends]],
  ["map", forgetUserMap, [mapKeys.bumps("ex"), mapKeys.gatherings]],
  ["places", forgetUserPlaces, [placeKeys.place("cafe")]],
  ["plans", forgetUserPlans, [planKeys.freeStatus("ex"), planKeys.birthdays]],
  ["profile", forgetUserProfile, [profileKeys.byId("ex")]],
  ["quest", forgetUserQuest, [questKeys.streak("ex"), questKeys.streakList]],
  ["school", forgetUserSchool, [schoolKeys.friendTimetable("ex"), schoolKeys.studying]],
  ["spaces", forgetUserSpaces, [spacesKeys.hangout("h"), spacesKeys.friendsInRooms]],
  ["video", forgetUserVideo, [videoKeys.home]],
];

describe.each(AREAS)("forgetting a person in %s", (_area, forget, keys) => {
  it.each(keys.map((key) => [JSON.stringify(key), key] as const))(
    "%s on screen is read again, and a read in flight does not bring them back",
    async (_name, queryKey) => {
      const queryClient = new QueryClient();
      queryClient.setQueryData(queryKey, "theirs");
      const answers: ((value: string) => void)[] = [];
      const stop = new QueryObserver(queryClient, {
        queryKey,
        queryFn: () => new Promise<string>((resolve) => answers.push(resolve)),
        staleTime: 0,
      }).subscribe(() => {});
      expect(answers).toHaveLength(1);
      const forgotten = forget(queryClient, "ex");
      // The read begun before the unfriend answers with their content; the one after, without.
      answers[0]?.("theirs");
      answers[1]?.("without");
      await forgotten;
      expect(queryClient.getQueryData(queryKey)).toBe("without");
      stop();
    },
  );

  it.each(keys.map((key) => [JSON.stringify(key), key] as const))(
    "%s off screen is not kept to be shown again",
    async (_name, queryKey) => {
      const queryClient = new QueryClient();
      queryClient.setQueryData(queryKey, "theirs");
      await forget(queryClient, "ex");
      expect(queryClient.getQueryData(queryKey)).toBeUndefined();
    },
  );
});
