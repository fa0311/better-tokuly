import { QueryClient } from "@tanstack/react-query";
import { create, notes } from "@tokuly/api";
import { storyKeys } from "@tokuly/core/stories";
import { seenMarker } from "../seen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));

const story = (id: string, viewed = false) =>
  create(notes.StorySchema, { id, viewed, owner: { id: "friend" }, items: [{}] });

function setUp(markStoryViewed = jest.fn(async () => ({}))) {
  const queryClient = new QueryClient();
  queryClient.setQueryData(storyKeys.tray, [
    create(notes.StoryReelSchema, {
      user: { id: "friend" },
      stories: [story("s1")],
      hasUnseen: true,
    }),
  ]);
  const mark = seenMarker(queryClient, () => ({ markStoryViewed }) as never);
  return { queryClient, mark, markStoryViewed };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("story seen state", () => {
  it("marks a friend's story once, and the tray shows the reel seen", async () => {
    const { queryClient, mark, markStoryViewed } = setUp();
    mark(story("s1"), false);
    mark(story("s1"), false);
    await flush();
    expect(markStoryViewed).toHaveBeenCalledTimes(1);
    expect(markStoryViewed).toHaveBeenCalledWith({ storyId: "s1" });
    const [reel] = queryClient.getQueryData<notes.StoryReel[]>(storyKeys.tray) ?? [];
    expect(reel?.hasUnseen).toBe(false);
    expect(reel?.stories[0]?.viewed).toBe(true);
  });

  it("never marks the viewer's own story, nor one already watched", async () => {
    const { mark, markStoryViewed } = setUp();
    mark(story("mine"), true);
    mark(story("old", true), false);
    await flush();
    expect(markStoryViewed).not.toHaveBeenCalled();
  });

  it("tries again the next time it is shown when marking failed", async () => {
    const markStoryViewed = jest
      .fn()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue({});
    const { mark } = setUp(markStoryViewed);
    mark(story("s1"), false);
    await flush();
    mark(story("s1"), false);
    await flush();
    expect(markStoryViewed).toHaveBeenCalledTimes(2);
  });
});
