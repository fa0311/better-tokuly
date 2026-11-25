import { toJsonString } from "@bufbuild/protobuf";
import { create, shorts } from "@tokuly/api";
import type { Stay } from "@tokuly/core/shorts";
import { API_PATH, api } from "#/lib/api";
import { feedClient } from "./api";

export { type Stay, StayMeter } from "@tokuly/core/shorts";

/** `ReportPlayback` takes at most this many reports per call. */
const BATCH = 50;

type Report = Pick<shorts.PlaybackReport, "shortId" | "source" | "watchedMs" | "completions">;

let queue: Report[] = [];
let scheduled = false;

async function flush(send: (reports: Report[]) => Promise<unknown>): Promise<void> {
  scheduled = false;
  const reports = queue;
  queue = [];
  for (let offset = 0; offset < reports.length; offset += BATCH) {
    // A lost report is a lost view, not something to bother the viewer with.
    await send(reports.slice(offset, offset + BATCH)).catch(() => {});
  }
}

/**
 * Hands a finished stay to the server. Stays that end in the same moment (a
 * fast swipe past several shorts) travel in one request.
 */
export function reportStay(
  shortId: string,
  source: shorts.PlaybackSource,
  stay: Stay,
  send: (reports: Report[]) => Promise<unknown> = (reports) =>
    feedClient().reportPlayback({ reports }),
): void {
  queue.push({
    shortId,
    source,
    watchedMs: BigInt(stay.watchedMs),
    completions: stay.completions,
  });
  if (scheduled) return;
  scheduled = true;
  queueMicrotask(() => void flush(send));
}

/**
 * The same for a stay that ends because the page is going away (closed,
 * reloaded, left for another site): an ordinary request would be dropped
 * with the page, so this one is marked to outlive it.
 */
export function reportStayOnLeave(
  shortId: string,
  source: shorts.PlaybackSource,
  stay: Stay,
): void {
  const token = api().session.peekAccessToken();
  const body = toJsonString(
    shorts.ReportPlaybackRequestSchema,
    create(shorts.ReportPlaybackRequestSchema, {
      reports: [
        { shortId, source, watchedMs: BigInt(stay.watchedMs), completions: stay.completions },
      ],
    }),
  );
  void fetch(
    `${window.location.origin}${API_PATH}/tokuly.shorts.v1.ShortFeedService/ReportPlayback`,
    {
      method: "POST",
      keepalive: true,
      credentials: "include",
      headers: {
        "content-type": "application/json",
        "x-tokuly-client": "web",
        ...(token ? { authorization: `Bearer ${token}` } : {}),
      },
      body,
    },
  ).catch(() => {});
}
