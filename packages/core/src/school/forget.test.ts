import { QueryClient } from "@tanstack/query-core";
import { describe, expect, it } from "vitest";
import { forgetUserSchool } from "./forget";
import { schoolKeys } from "./keys";

describe("forgetting a friend in school", () => {
  it("drops their timetable and studying, and keeps the viewer's own and other friends'", async () => {
    const queryClient = new QueryClient();
    const theirs = [
      schoolKeys.friendTimetable("aoi", "t1"),
      schoolKeys.day("2026-11-29", "aoi"),
      schoolKeys.days("2026-11-23", "2026-11-29", "aoi"),
      schoolKeys.now("aoi"),
      schoolKeys.heatmap("aoi"),
    ];
    const kept = [schoolKeys.day("2026-11-29"), schoolKeys.now("ren"), schoolKeys.heatmap()];
    for (const key of [...theirs, ...kept]) queryClient.setQueryData(key, {});
    await forgetUserSchool(queryClient, "aoi");
    for (const key of theirs) expect(queryClient.getQueryData(key)).toBeUndefined();
    for (const key of kept) expect(queryClient.getQueryData(key)).toBeDefined();
  });
});
