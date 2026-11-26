import { errorReason, type video } from "@tokuly/api";
import { PlaybackReporter } from "@tokuly/core/video";
import { useEventListener } from "expo";
import type { VideoPlayer } from "expo-video";
import { useEffect, useRef } from "react";
import { AppState } from "react-native";
import { playbackClient } from "./data";

/** A jump bigger than this between two time updates is a seek, reported at once. */
const SEEK_SECONDS = 3;

/**
 * Reports one viewing of a video the way the API asks
 * (`@tokuly/core/video`'s `PlaybackReporter`): a session opens when playback
 * first starts, the position goes out at the server's cadence while playing
 * and at once on pause, seek, end, when the app leaves the front and when the
 * screen closes. That counts the view, feeds watch time and moves the resume
 * position.
 *
 * The client is the account's that opened the screen, taken once: a report
 * sent as the screen closes after a switch still speaks for the viewer who
 * watched. A video in picture-in-picture is still being watched.
 */
export function usePlaybackReport({
  videoId,
  player,
  source,
  onViewCount,
}: {
  /** Nothing is reported while it is undefined (the video cannot be played). */
  videoId: string | undefined;
  player: VideoPlayer;
  source: video.TrafficSource;
  onViewCount?(count: bigint): void;
}): void {
  const reporter = useRef<PlaybackReporter<video.TrafficSource> | null>(null);
  const viewCount = useRef(onViewCount);
  useEffect(() => {
    viewCount.current = onViewCount;
  });
  const lastTime = useRef(0);

  useEffect(() => {
    if (!videoId) return;
    const client = playbackClient();
    const current = new PlaybackReporter(videoId, source, {
      startPlayback: (request) => client.startPlayback(request),
      reportPlayback: (request) => client.reportPlayback(request),
      position: () => read(() => player.currentTime, lastTime.current),
      isPlaying: () => read(() => player.playing, false),
      isSessionGone: (error) => errorReason(error) === "PLAYBACK_SESSION_NOT_FOUND",
      onViewCount: (count) => viewCount.current?.(count),
    });
    reporter.current = current;
    // Leaving the front: where the viewer is now (picture-in-picture goes on reporting).
    const appState = AppState.addEventListener("change", (state) => {
      if (state !== "active") void current.report();
    });
    return () => {
      appState.remove();
      // The request leaves before `dispose`; only its answer is dropped.
      void current.report();
      current.dispose();
      reporter.current = null;
    };
  }, [videoId, source, player]);

  useEventListener(player, "playingChange", ({ isPlaying }) => {
    if (isPlaying) reporter.current?.play();
    else void reporter.current?.report();
  });
  useEventListener(player, "playToEnd", () => void reporter.current?.report(true));
  useEventListener(player, "timeUpdate", ({ currentTime }) => {
    if (Math.abs(currentTime - lastTime.current) > SEEK_SECONDS) void reporter.current?.report();
    lastTime.current = currentTime;
  });
}

/** The player may already be released when the screen's last report is sent. */
function read<T>(value: () => T, fallback: T): T {
  try {
    return value();
  } catch {
    return fallback;
  }
}
