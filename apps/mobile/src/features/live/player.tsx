import type { live } from "@tokuly/api";
import { useEventListener } from "expo";
import { Image } from "expo-image";
import { useVideoPlayer, VideoView } from "expo-video";
import { Volume2, VolumeX } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ActivityIndicator, Pressable, Text, View } from "react-native";
import { mediaUrl } from "#/lib/config";

/** How long to wait before opening the media again after the n-th failure in a row. */
export function retryDelay(failures: number): number {
  return Math.min(15_000, 1000 * 2 ** failures);
}

/** How far behind the live edge a low-latency stream aims to play (iOS honours it). */
const LOW_LATENCY_OFFSET_SECONDS = 2;

/**
 * The picture of a stream that is on air, as Low-Latency HLS: the media
 * server's `hls_url`, which AVPlayer (iOS) and ExoPlayer (Android) play
 * natively, partial segments included. (The phone app has no WebRTC stack,
 * so WHEP is the web's.)
 *
 * One media session is opened per `attempt`: fresh playback URLs alone do
 * not open a new one (the one that plays keeps working). When playback
 * fails — the playlist of a stream that just went on air is not there yet,
 * the token ran out, the viewer lost access — `onSessionLost` lets the
 * screen read the stream again, and the media is opened again a moment
 * later with the URLs the cache holds by then.
 */
export function LivePlayer({
  playback,
  lowLatency,
  poster,
  onSessionLost,
}: {
  playback: live.Playback;
  lowLatency: boolean;
  poster?: string;
  onSessionLost(): void;
}) {
  const { t } = useTranslation("live");
  // The newest URLs and callback, read when a session opens (not a reason to open one).
  const latest = useRef({ playback, onSessionLost });
  latest.current = { playback, onSessionLost };
  const [attempt, setAttempt] = useState(0);
  const failures = useRef(0);
  const [muted, setMuted] = useState(false);
  const [waiting, setWaiting] = useState(true);
  const [offset, setOffset] = useState<number | null>(null);

  const player = useVideoPlayer(null, (created) => {
    created.staysActiveInBackground = false;
  });

  // biome-ignore lint/correctness/useExhaustiveDependencies: `attempt` is what opens a new session.
  useEffect(() => {
    const uri = mediaUrl(latest.current.playback.hlsUrl);
    if (!uri) return;
    setWaiting(true);
    void player.replaceAsync({ uri, contentType: "hls" }).then(() => {
      if (lowLatency) player.targetOffsetFromLive = LOW_LATENCY_OFFSET_SECONDS;
      player.play();
    });
  }, [player, attempt, lowLatency]);

  useEffect(() => {
    player.muted = muted;
  }, [player, muted]);

  // How far behind live the picture is, once a second.
  useEffect(() => {
    const timer = setInterval(() => setOffset(player.currentOffsetFromLive ?? null), 1000);
    return () => clearInterval(timer);
  }, [player]);

  const retry = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(retry.current), []);

  useEventListener(player, "statusChange", ({ status }) => {
    if (status === "readyToPlay") {
      failures.current = 0;
      setWaiting(false);
    } else if (status === "loading") {
      setWaiting(true);
    } else if (status === "error") {
      setWaiting(true);
      latest.current.onSessionLost();
      clearTimeout(retry.current);
      retry.current = setTimeout(
        () => setAttempt((value) => value + 1),
        retryDelay(failures.current),
      );
      failures.current += 1;
    }
  });

  return (
    <View testID="live-player" className="aspect-video w-full bg-black">
      <VideoView player={player} style={{ flex: 1 }} contentFit="contain" nativeControls={false} />
      {waiting && (
        <View className="absolute inset-0 items-center justify-center">
          {poster && (
            <Image
              source={poster}
              style={{ position: "absolute", inset: 0, opacity: 0.4 }}
              contentFit="cover"
            />
          )}
          <ActivityIndicator color="#ffffff" />
        </View>
      )}
      <View className="absolute right-2 bottom-2 flex-row items-center gap-2">
        {offset !== null && !waiting && (
          <Text
            testID="live-latency"
            className="rounded-full bg-black/55 px-2 py-1 text-[11px] font-bold text-white"
          >
            {offset < 1.5
              ? t("player.latencyRealtime")
              : t("player.latencySeconds", { count: Math.round(offset) })}
          </Text>
        )}
        <Pressable
          testID="live-sound"
          accessibilityRole="button"
          accessibilityLabel={t(muted ? "player.soundOn" : "player.soundOff")}
          onPress={() => setMuted(!muted)}
          className="rounded-full bg-black/55 p-2"
        >
          {muted ? <VolumeX size={18} color="#ffffff" /> : <Volume2 size={18} color="#ffffff" />}
        </Pressable>
      </View>
    </View>
  );
}
