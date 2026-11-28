import type { Client } from "@connectrpc/connect";
import { QueryClient } from "@tanstack/query-core";
import { create, fromDate, notes } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { addStory, forgetUserStories, patchStory, removeStory } from "./cache";
import { storyKeys } from "./keys";
import { storyQueries } from "./queries";

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

/** A server whose story tray and reels the test hands out, one answer per read. */
function server() {
  const trays: ((reels: notes.StoryReel[]) => void)[] = [];
  const reels: ((stories: notes.Story[]) => void)[] = [];
  const client = () =>
    ({
      getStoryTray: () =>
        new Promise((resolve) => trays.push((value) => resolve({ reels: value }))),
      listUserStories: () =>
        new Promise((resolve) => reels.push((value) => resolve({ stories: value }))),
    }) as unknown as Client<typeof notes.StoryService>;
  return {
    client,
    tray: (read: number, value: notes.StoryReel[]) => trays[read]?.(value),
    reel: (read: number, value: notes.Story[]) => reels[read]?.(value),
  };
}

describe("a read in flight when a story or a person goes", () => {
  it("does not bring a deleted story back into the tray", async () => {
    const queryClient = withTray(reel("aoi", [story("a1", "aoi", 1), story("a2", "aoi", 2)]));
    const api = server();
    // The tray is read again (a Welcome); the story is deleted before the answer, made earlier, lands.
    const reading = queryClient.fetchQuery({ ...storyQueries.tray(api.client), staleTime: 0 });
    removeStory(queryClient, "a1");
    api.tray(0, [reel("aoi", [story("a1", "aoi", 1), story("a2", "aoi", 2)])]);
    await reading;
    expect(tray(queryClient)[0]?.stories.map((item) => item.id)).toEqual(["a2"]);
  });

  it("does not bring a forgotten person's reel back, in the tray or on their profile", async () => {
    const queryClient = new QueryClient();
    const api = server();
    const reading = queryClient.fetchQuery(storyQueries.tray(api.client));
    const theirs = queryClient.fetchQuery(storyQueries.user(api.client, "ren"));
    forgetUserStories(queryClient, "ren");
    api.tray(0, [reel(ME, [story("m1", ME, 1)]), reel("ren", [story("r1", "ren", 2)])]);
    api.reel(0, [story("r1", "ren", 2)]);
    await reading;
    expect(tray(queryClient).map((item) => item.user?.id)).toEqual([ME]);
    // Their reel is thrown away, and its read with it.
    await expect(theirs).rejects.toThrow();
    expect(queryClient.getQueryData(storyKeys.user("ren"))).toBeUndefined();
  });

  it("keeps a story posted while the tray was read", async () => {
    const queryClient = withTray();
    const api = server();
    const reading = queryClient.fetchQuery({ ...storyQueries.tray(api.client), staleTime: 0 });
    addStory(queryClient, story("a1", "aoi", 10), ME);
    api.tray(0, []);
    await reading;
    expect(tray(queryClient).map((item) => item.user?.id)).toEqual(["aoi"]);
  });
});
