import { QueryClient } from "@tanstack/query-core";
import { create, fromDate, notes } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { addStory, forgetUserStories, patchStory, removeStory } from "./cache";
import { storyKeys } from "./keys";

const ME = "me";
const user = (id: string) => ({ id, handle: id, displayName: id });
const story = (id: string, owner: string, minute: number, viewed = false) =>
  create(notes.StorySchema, {
    id,
    owner: user(owner),
    viewed,
    createdAt: fromDate(new Date(2026, 10, 25, 12, minute)),
  });
const reel = (owner: string, stories: notes.Story[]) =>
  create(notes.StoryReelSchema, {
    user: user(owner),
    stories,
    hasUnseen: stories.some((item) => !item.viewed),
    latestAt: stories.at(-1)?.createdAt,
  });

function withTray(...reels: notes.StoryReel[]): QueryClient {
  const queryClient = new QueryClient();
  queryClient.setQueryData(storyKeys.tray, reels);
  return queryClient;
}

const tray = (queryClient: QueryClient) =>
  queryClient.getQueryData<notes.StoryReel[]>(storyKeys.tray) ?? [];

describe("story cache", () => {
  it("puts a friend's new story in their reel and keeps the viewer's own reel first", () => {
    const queryClient = withTray(
      reel(ME, [story("m1", ME, 1)]),
      reel("ren", [story("r1", "ren", 5, true)]),
    );
    addStory(queryClient, story("a1", "aoi", 10), ME);
    expect(tray(queryClient).map((item) => item.user?.id)).toEqual([ME, "aoi", "ren"]);
    expect(tray(queryClient)[1]?.hasUnseen).toBe(true);
  });

  it("marks a reel seen once every story in it was viewed", () => {
    const queryClient = withTray(reel("aoi", [story("a1", "aoi", 1)]));
    patchStory(queryClient, "a1", (item) => ({ ...item, viewed: true }));
    expect(tray(queryClient)[0]?.hasUnseen).toBe(false);
  });

  it("drops a reel left without stories, and everything of a person forgotten", () => {
    const queryClient = withTray(
      reel("aoi", [story("a1", "aoi", 1)]),
      reel("ren", [story("r1", "ren", 2)]),
    );
    removeStory(queryClient, "a1");
    expect(tray(queryClient).map((item) => item.user?.id)).toEqual(["ren"]);
    queryClient.setQueryData(storyKeys.story("r1"), story("r1", "ren", 2));
    forgetUserStories(queryClient, "ren");
    expect(tray(queryClient)).toEqual([]);
    expect(queryClient.getQueryData(storyKeys.story("r1"))).toBeUndefined();
  });
});
