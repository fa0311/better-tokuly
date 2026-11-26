import { create, shorts } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { listKey, loadListPage, type ShortClients } from "./lists";

const short = (id: string) => create(shorts.ShortSchema, { id });

function clients(calls: string[]): ShortClients {
  const answer = (name: string, response: unknown) => async (request: unknown) => {
    calls.push(`${name} ${JSON.stringify(request)}`);
    return response;
  };
  return {
    feed: () =>
      ({
        getForYouFeed: answer("forYou", {
          items: [{ short: short("a"), repeat: true }, {}],
          page: { nextCursor: "n" },
        }),
      }) as never,
    discovery: () =>
      ({
        listHashtagShorts: answer("hashtag", { shorts: [short("b")], page: { nextCursor: "" } }),
      }) as never,
    shorts: () => ({}) as never,
  };
}

describe("shorts lists", () => {
  it("reads the for-you feed ten at a time and keeps what it says about each short", async () => {
    const calls: string[] = [];
    const page = await loadListPage(clients(calls), { kind: "forYou" }, "c1");
    expect(calls).toEqual(['forYou {"page":{"limit":10,"cursor":"c1"}}']);
    // An item without a short (gone meanwhile) is left out.
    expect(page.entries.map((entry) => [entry.short.id, entry.repeat])).toEqual([["a", true]]);
    expect(page.page?.nextCursor).toBe("n");
  });

  it("reads a hashtag page as a grid, in the order asked for", async () => {
    const calls: string[] = [];
    const page = await loadListPage(clients(calls), { kind: "hashtag", key: "dance" }, "", {
      sort: shorts.HashtagSort.RECENT,
    });
    expect(calls).toEqual(['hashtag {"tag":"dance","sort":2,"page":{"limit":24,"cursor":""}}']);
    expect(page.entries.map((entry) => entry.short.id)).toEqual(["b"]);
  });

  it("keys a link with the for-you feed it plays on into, and a grid as a list", () => {
    expect(listKey({ kind: "link" })).toEqual(listKey({ kind: "forYou" }));
    expect(listKey({ kind: "hashtag", key: "dance" })).toEqual([
      "shorts",
      "list",
      "hashtag",
      "dance",
      0,
      false,
    ]);
  });
});
