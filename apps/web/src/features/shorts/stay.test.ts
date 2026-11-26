import { shorts } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { reportStay } from "./stay";

describe("reportStay", () => {
  it("sends the stays of one moment together", async () => {
    const calls: { shortId: string; watchedMs: bigint }[][] = [];
    const send = async (reports: { shortId: string; watchedMs: bigint }[]) => {
      calls.push(reports);
    };
    const source = shorts.PlaybackSource.FOR_YOU;
    reportStay("a", source, { watchedMs: 1_200, completions: 0 }, send);
    reportStay("b", source, { watchedMs: 300, completions: 1 }, send);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toHaveLength(1);
    expect(calls[0]?.map((report) => [report.shortId, report.watchedMs])).toEqual([
      ["a", 1_200n],
      ["b", 300n],
    ]);
  });
});
