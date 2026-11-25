import { QueryClient } from "@tanstack/query-core";
import { create, shorts } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { patchViewer, removeShortEverywhere } from "./cache";
import { shortKeys } from "./keys";

const short = (id: string) =>
  create(shorts.ShortSchema, { id, shortId: `s-${id}`, viewer: { saved: false } });

function cached() {
  const queryClient = new QueryClient();
  const feed = ["shorts", "feed", "forYou", "", 0, false];
  queryClient.setQueryData(feed, {
    pages: [{ entries: [{ short: short("a") }, { short: short("b") }] }],
    pageParams: [""],
  });
  queryClient.setQueryData(shortKeys.short("s-a"), short("a"));
  return { queryClient, feed };
}

type Feed = { pages: { entries: { short: shorts.Short }[] }[] };

describe("shorts cache", () => {
  it("changes a short wherever it is held, by UUID or short ID", () => {
    const { queryClient, feed } = cached();
    patchViewer(queryClient, "s-a", { saved: true }, { saveCount: 3n });
    const entries = queryClient.getQueryData<Feed>(feed)?.pages[0]?.entries;
    expect(entries?.[0]?.short.viewer?.saved).toBe(true);
    expect(entries?.[0]?.short.saveCount).toBe(3n);
    expect(entries?.[1]?.short.viewer?.saved).toBe(false);
    expect(queryClient.getQueryData<shorts.Short>(shortKeys.short("s-a"))?.saveCount).toBe(3n);
  });

  it("drops a short and its feed entry from every list, but not its own entry", () => {
    const { queryClient, feed } = cached();
    removeShortEverywhere(queryClient, "a");
    const entries = queryClient.getQueryData<Feed>(feed)?.pages[0]?.entries;
    expect(entries?.map((entry) => entry.short.id)).toEqual(["b"]);
    expect(queryClient.getQueryData(shortKeys.short("s-a"))).toBeDefined();
  });
});
