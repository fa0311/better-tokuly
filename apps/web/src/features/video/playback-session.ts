import { errorReason, video } from "@tokuly/api";
import { type RefObject, useEffect, useRef } from "react";
import { API_PATH, api } from "#/lib/api";
import { playbackClient } from "./api";
import { PlaybackReporter } from "./playback-reporter";
import type { PlayerHandle } from "./player/player";

const { TrafficSource } = video;

/** An announcement is about the navigation that follows it at once, not about a visit minutes later. */
const ANNOUNCEMENT_MS = 10_000;
let announced: { source: video.TrafficSource; atMs: number } | undefined;

/**
 * Remembers where the viewer is coming from for the watch page they are
 * about to open (a card in the home feed, "up next", a playlist…). Call it
 * when the link is followed.
 */
export function announceTrafficSource(source: video.TrafficSource): void {
  announced = { source, atMs: Date.now() };
}

/** The source of the visit that is starting; without a fresh announcement the page was opened directly. */
export function takeTrafficSource(): video.TrafficSource {
  return announced && Date.now() - announced.atMs < ANNOUNCEMENT_MS ? announced.source : TrafficSource.DIRECT;
}

/** The player events playback reporting listens to; pass them to `<VideoPlayer>`. */
export interface PlaybackEvents {
  onPlay(): void;
  onPause(): void;
  onSeeked(): void;
  onEnded(): void;
  onWaiting(waiting: boolean): void;
}

/**
 * Reports playback of a video the way the API asks: a session is opened when
 * playback first starts, then the position is reported at the server's
 * cadence while playing and at once on pause, seek, end, when the tab is
 * hidden and when the page is left. That is what counts views, feeds watch
 * time and remembers where to resume.
 */
export function usePlaybackSession({
  videoId,
  player,
  source,
  onViewCount,
}: {
  /** The video's ID; nothing is reported while it is undefined. */
  videoId: string | undefined;
  player: RefObject<PlayerHandle | null>;
  source: video.TrafficSource;
  onViewCount?(count: bigint): void;
}): PlaybackEvents {
  const reporter = useRef<PlaybackReporter<video.TrafficSource> | null>(null);
  const waiting = useRef(false);
  const viewCount = useRef(onViewCount);
  useEffect(() => {
    viewCount.current = onViewCount;
  });

  // The player is gone by the time the page's last report is sent, so the position is kept here.
  const lastPosition = useRef(0);
  const position = () => {
    if (player.current) lastPosition.current = player.current.currentTime();
    return lastPosition.current;
  };

  // One reporter per viewing. Leaving ends it — also a session that was still being opened.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `position` reads live refs; the reporter is rebuilt per video only.
  useEffect(() => {
    if (!videoId) return;
    const current = new PlaybackReporter(videoId, source, {
      startPlayback: (request) => playbackClient().startPlayback(request),
      reportPlayback: (request) => playbackClient().reportPlayback(request),
      position,
      isPlaying: () => Boolean(player.current && !player.current.paused() && !waiting.current && document.visibilityState === "visible"),
      isSessionGone: (error) => errorReason(error) === "PLAYBACK_SESSION_NOT_FOUND",
      onViewCount: (count) => viewCount.current?.(count),
    });
    reporter.current = current;

    // Leaving the tab or the page: one last report that survives the page going away.
    const beacon = () => {
      const sessionId = current.sessionId;
      if (!sessionId) return;
      const token = api().session.peekAccessToken();
      void fetch(`${API_PATH}/tokuly.video.v1.PlaybackService/ReportPlayback`, {
        method: "POST",
        keepalive: true,
        credentials: "include",
        headers: { "content-type": "application/json", "x-tokuly-client": "web", ...(token ? { authorization: `Bearer ${token}` } : {}) },
        body: JSON.stringify({ sessionId, positionSeconds: position(), playing: false, ended: false }),
      }).catch(() => {});
    };
    const onVisibility = () => {
      if (document.visibilityState === "hidden") beacon();
      else void current.report();
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("pagehide", beacon);
    const tracker = setInterval(position, 1000);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pagehide", beacon);
      clearInterval(tracker);
      // Moving on inside the app: say where the viewer stopped.
      beacon();
      current.dispose();
      reporter.current = null;
    };
  }, [videoId, source]);

  return {
    onPlay: () => reporter.current?.play(),
    onPause: () => void reporter.current?.report(),
    onSeeked: () => void reporter.current?.report(),
    onEnded: () => void reporter.current?.report(true),
    onWaiting: (stalled) => {
      waiting.current = stalled;
    },
  };
}
