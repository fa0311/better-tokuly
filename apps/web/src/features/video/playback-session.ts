import { errorReason, video } from "@tokuly/api";
import { type RefObject, useEffect, useRef } from "react";
import { API_PATH, api } from "#/lib/api";
import { playbackClient } from "./api";
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

interface Session {
  id: string;
  intervalMs: number;
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
  const session = useRef<Session | null>(null);
  const starting = useRef(false);
  const waiting = useRef(false);
  const timer = useRef<ReturnType<typeof setInterval> | undefined>(undefined);
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
  const isPlaying = () => Boolean(player.current && !player.current.paused() && !waiting.current && document.visibilityState === "visible");

  const report = async (ended = false) => {
    const current = session.current;
    if (!current) return;
    try {
      const response = await playbackClient().reportPlayback({ sessionId: current.id, positionSeconds: position(), playing: !ended && isPlaying(), ended });
      viewCount.current?.(response.viewCount);
    } catch (error) {
      // The session expired (or the video is gone): the next play opens a new one.
      if (errorReason(error) === "PLAYBACK_SESSION_NOT_FOUND" && session.current === current) session.current = null;
    }
  };

  const start = async () => {
    if (!videoId || session.current || starting.current) return;
    starting.current = true;
    try {
      const response = await playbackClient().startPlayback({ videoId, source, positionSeconds: position() });
      session.current = { id: response.sessionId, intervalMs: Math.max(2, response.reportIntervalSeconds || 10) * 1000 };
      viewCount.current?.(response.viewCount);
      clearInterval(timer.current);
      timer.current = setInterval(() => {
        // A session that lost its ID is reopened by the next tick that finds the video playing.
        if (session.current) void report();
        else if (isPlaying()) void start();
      }, session.current.intervalMs);
    } catch {
      // Not ready, age-restricted or rate-limited: watching goes on unreported.
    } finally {
      starting.current = false;
    }
  };

  // Leaving the tab or the page: one last report that survives the page going away.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the handlers read live refs; they are rebuilt per video only.
  useEffect(() => {
    const beacon = () => {
      const current = session.current;
      if (!current) return;
      const token = api().session.peekAccessToken();
      void fetch(`${API_PATH}/tokuly.video.v1.PlaybackService/ReportPlayback`, {
        method: "POST",
        keepalive: true,
        credentials: "include",
        headers: { "content-type": "application/json", "x-tokuly-client": "web", ...(token ? { authorization: `Bearer ${token}` } : {}) },
        body: JSON.stringify({ sessionId: current.id, positionSeconds: position(), playing: false, ended: false }),
      }).catch(() => {});
    };
    const onVisibility = () => {
      if (document.visibilityState === "hidden") beacon();
      else void report();
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("pagehide", beacon);
    const tracker = setInterval(position, 1000);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pagehide", beacon);
      clearInterval(tracker);
      clearInterval(timer.current);
      // Moving on inside the app: say where the viewer stopped.
      beacon();
      session.current = null;
    };
  }, [videoId]);

  return {
    onPlay: () => {
      if (session.current) void report();
      else void start();
    },
    onPause: () => void report(),
    onSeeked: () => void report(),
    onEnded: () => void report(true),
    onWaiting: (stalled) => {
      waiting.current = stalled;
    },
  };
}
