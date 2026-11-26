import { errorReason, video } from "@tokuly/api";
import { PlaybackReporter } from "@tokuly/core/video";
import { useEventListener } from "expo";
import type { VideoPlayer } from "expo-video";
import { useEffect, useRef } from "react";
import { AppState } from "react-native";
import { type Account, currentAccount, isActive } from "#/features/session/account";

/** A jump bigger than this between two time updates is a seek, reported at once. */
const SEEK_SECONDS = 3;

/** A video being watched: whose viewing it is, and how it ends. */
interface Viewing {
  account: Account;
  /** Says where the viewer stopped (as long as the account is the one in use) and reports nothing more. */
  close(): Promise<void>;
}

const viewings = new Set<Viewing>();

/**
 * Ends the reporting of what `account` is watching, before it leaves the
 * device (`lifecycle.ts`): the last position is sent while its session still
 * works. Afterwards its API object would ask for a new token with whatever
 * the device keeps for the next account.
 */
export async function closeViewings(account: Account): Promise<void> {
  const leaving = [...viewings].filter((viewing) => viewing.account.api === account.api);
  await Promise.all(leaving.map((viewing) => viewing.close()));
}

/** The view's events playback reporting listens to; pass them to `<VideoView>`. */
export interface PictureEvents {
  onPictureInPictureStart(): void;
  onPictureInPictureStop(): void;
}

/**
 * Reports one viewing of a video the way the API asks
 * (`@tokuly/core/video`'s `PlaybackReporter`): a session opens when playback
 * first starts, the position goes out at the server's cadence while playing
 * and at once on pause, seek, end, when the picture leaves the screen or
 * comes back, and when the screen closes. That counts the view, feeds watch
 * time and moves the resume position.
 *
 * Watching is a picture on the screen: the app in front, or its small window
 * (picture-in-picture). Sound that goes on behind another app or a locked
 * screen is reported as not playing, like a hidden tab on the web.
 *
 * The viewing belongs to the account that opened the screen: every report is
 * made with its API, and the last one when it leaves (`closeViewings`) or
 * when the screen closes while it is still the one in use.
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
}): PictureEvents {
  const reporter = useRef<PlaybackReporter<video.TrafficSource> | null>(null);
  const viewCount = useRef(onViewCount);
  useEffect(() => {
    viewCount.current = onViewCount;
  });
  const lastTime = useRef(0);
  const inPicture = useRef(false);
  const changed = (playing?: boolean) => {
    if (reporter.current) tell(reporter.current, isWatching(player, inPicture.current, playing));
  };

  useEffect(() => {
    const account = currentAccount();
    if (!videoId || !account) return;
    const client = account.api.service(video.PlaybackService);
    const current = new PlaybackReporter(videoId, source, {
      startPlayback: (request) => client.startPlayback(request),
      reportPlayback: (request) => client.reportPlayback(request),
      position: () => read(() => player.currentTime, lastTime.current),
      isPlaying: () => isWatching(player, inPicture.current),
      isSessionGone: (error) => errorReason(error) === "PLAYBACK_SESSION_NOT_FOUND",
      onViewCount: (count) => viewCount.current?.(count),
    });
    const viewing: Viewing = {
      account,
      async close() {
        viewings.delete(viewing);
        // The request leaves before `dispose`; only its answer is dropped.
        const last = isActive(account) ? current.report() : undefined;
        current.dispose();
        await last;
      },
    };
    viewings.add(viewing);
    reporter.current = current;
    const appState = AppState.addEventListener("change", () =>
      tell(current, isWatching(player, inPicture.current)),
    );
    return () => {
      appState.remove();
      void viewing.close();
      reporter.current = null;
    };
  }, [videoId, source, player]);

  useEventListener(player, "playingChange", ({ isPlaying }) => changed(isPlaying));
  useEventListener(player, "playToEnd", () => void reporter.current?.report(true));
  useEventListener(player, "timeUpdate", ({ currentTime }) => {
    if (Math.abs(currentTime - lastTime.current) > SEEK_SECONDS) void reporter.current?.report();
    lastTime.current = currentTime;
  });

  const picture = (shows: boolean) => {
    inPicture.current = shows;
    changed();
  };
  return {
    onPictureInPictureStart: () => picture(true),
    onPictureInPictureStop: () => picture(false),
  };
}

/** The picture moves in front of the viewer: in the app, or in its small window. */
function isWatching(
  player: VideoPlayer,
  inPicture: boolean,
  playing = read(() => player.playing, false),
): boolean {
  return playing && (AppState.currentState === "active" || inPicture);
}

/** Whether the viewer is watching may have changed: watching opens the session or goes on in it. */
function tell(reporter: PlaybackReporter<video.TrafficSource>, watching: boolean): void {
  if (watching) reporter.play();
  else void reporter.report();
}

/** The player may already be released when the screen's last report is sent. */
function read<T>(value: () => T, fallback: T): T {
  try {
    return value();
  } catch {
    return fallback;
  }
}
