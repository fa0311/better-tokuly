import { QueryClient } from "@tanstack/query-core";
import { create, video } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { patchVideoEverywhere, removeVideoEverywhere } from "./cache";
import { videoKeys } from "./keys";

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
});
