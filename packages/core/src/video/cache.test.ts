import { QueryClient } from "@tanstack/query-core";
import { create, video } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { patchVideoEverywhere, rateVideo, removeVideoEverywhere } from "./cache";
import { videoKeys } from "./keys";

const { Rating } = video;

/** The watch page's video as the server has it: liked by the viewer or not. */
const watched = (liked: boolean) =>
  create(video.VideoSchema, {
    summary: { id: "a", likeCount: liked ? 5n : 4n },
    viewer: { rating: liked ? Rating.LIKE : Rating.NONE },
  });

const liking = (queryClient: QueryClient, send = async () => watched(true)) =>
  rateVideo(queryClient, "a", Rating.LIKE, async () => {
    const answer = await send();
    return { rating: Rating.LIKE, likeCount: answer.summary?.likeCount ?? 0n };
  });

const card = (id: string, title = id) =>
  create(video.VideoSummarySchema, { id, shortId: `s-${id}`, title });

describe("video cache edits", () => {
  it("patches a card in every list and keeps the rest as it was", () => {
    const queryClient = new QueryClient();
    const other = card("b");
    queryClient.setQueryData(videoKeys.home, { pages: [{ videos: [card("a"), other] }] });
    queryClient.setQueryData(videoKeys.history, { pages: [{ items: [{ video: card("a") }] }] });
    patchVideoEverywhere(queryClient, "s-a", (summary) => ({ ...summary, title: "new" }));
    const home = queryClient.getQueryData<{ pages: { videos: video.VideoSummary[] }[] }>(
      videoKeys.home,
    );
    expect(home?.pages[0]?.videos.map((summary) => summary.title)).toEqual(["new", "b"]);
    expect(home?.pages[0]?.videos[1]).toBe(other);
    const history = queryClient.getQueryData<{
      pages: { items: { video: video.VideoSummary }[] }[];
    }>(videoKeys.history);
    expect(history?.pages[0]?.items[0]?.video.title).toBe("new");
  });

  it("drops a removed card and the row wrapping it", () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData(videoKeys.home, { pages: [{ videos: [card("a"), card("b")] }] });
    queryClient.setQueryData(videoKeys.playlistItems("WL"), {
      pages: [{ items: [{ video: card("a") }, { video: card("c") }] }],
    });
    removeVideoEverywhere(queryClient, "a");
    expect(
      queryClient
        .getQueryData<{ pages: { videos: video.VideoSummary[] }[] }>(videoKeys.home)
        ?.pages[0]?.videos.map((summary) => summary.id),
    ).toEqual(["b"]);
    expect(
      queryClient
        .getQueryData<{ pages: { items: { video: video.VideoSummary }[] }[] }>(
          videoKeys.playlistItems("WL"),
        )
        ?.pages[0]?.items.map((item) => item.video.id),
    ).toEqual(["c"]);
  });

  it("keeps a like when a read from before it lands afterwards", async () => {
    const queryClient = new QueryClient();
    const key = videoKeys.video("a");
    queryClient.setQueryData(key, watched(false));
    // The page is being read again (a focus, an invalidation) when the thumb is tapped.
    let land = (_item: video.Video) => {};
    const read = queryClient
      .fetchQuery({ queryKey: key, queryFn: () => new Promise<video.Video>((r) => (land = r)) })
      .catch(() => undefined);
    await liking(queryClient);
    land(watched(false));
    await read;
    const item = queryClient.getQueryData<video.Video>(key);
    expect(item?.viewer?.rating).toBe(Rating.LIKE);
    expect(item?.summary?.likeCount).toBe(5n);
  });

  it("shows a rating at once and takes only it back when the server refuses", async () => {
    const queryClient = new QueryClient();
    const key = videoKeys.video("a");
    queryClient.setQueryData(key, watched(false));
    let refuse = (_error: Error) => {};
    const rated = liking(
      queryClient,
      () => new Promise<video.Video>((_resolve, reject) => (refuse = reject)),
    );
    await new Promise((resolve) => setTimeout(resolve));
    expect(queryClient.getQueryData<video.Video>(key)?.summary?.likeCount).toBe(5n);
    // Views counted meanwhile stay.
    queryClient.setQueryData<video.Video>(key, (current) =>
      current?.summary ? { ...current, summary: { ...current.summary, viewCount: 9n } } : current,
    );
    refuse(new Error("RATE_LIMITED"));
    await expect(rated).rejects.toThrow("RATE_LIMITED");
    const item = queryClient.getQueryData<video.Video>(key);
    expect(item?.viewer?.rating).toBe(Rating.NONE);
    expect(item?.summary?.likeCount).toBe(4n);
    expect(item?.summary?.viewCount).toBe(9n);
  });
});
