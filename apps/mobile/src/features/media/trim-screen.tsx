import { durationLimit } from "@tokuly/core/errors";
import { clock } from "@tokuly/core/shorts";
import { useVideoPlayer, VideoView } from "expo-video";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal, PanResponder, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { ErrorNote } from "#/components/page";
import i18n from "#/lib/i18n";
import { trimFile } from "#/platform/trim";
import type { LocalMedia } from "./posts";

/** Where the kept part starts, given where the window was dragged to: inside the video, never past its end. */
export function clampStart(start: number, length: number, duration: number): number {
  return Math.min(Math.max(0, start), Math.max(0, duration - length));
}

/**
 * Cutting a video that is longer than the server takes: a preview of the
 * part that is kept, looping, and a window as long as the limit to drag
 * along the video. "Done" cuts it on the device (`platform/trim.ts`).
 */
export function TrimScreen({
  video,
  maxSeconds,
  onDone,
  onCancel,
}: {
  video: LocalMedia;
  maxSeconds: number;
  onDone(trimmed: LocalMedia): void;
  onCancel(): void;
}) {
  const { t } = useTranslation("mobile");
  const duration = video.duration ?? maxSeconds;
  const length = Math.min(maxSeconds, duration);
  const [start, setStart] = useState(0);
  const [track, setTrack] = useState(0);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const player = useVideoPlayer({ uri: video.uri }, (created) => {
    created.muted = false;
    created.play();
  });

  // The preview loops over the kept part.
  const range = useRef({ start, end: start + length });
  range.current = { start, end: start + length };
  useEffect(() => {
    player.currentTime = start;
  }, [player, start]);
  useEffect(() => {
    const timer = setInterval(() => {
      if (player.currentTime >= range.current.end || player.currentTime < range.current.start - 0.5)
        player.currentTime = range.current.start;
    }, 200);
    return () => clearInterval(timer);
  }, [player]);

  const windowWidth = track * (length / duration);
  const drag = useRef({ from: 0 });
  const live = useRef({ start, track, windowWidth });
  live.current = { start, track, windowWidth };
  const pan = useRef(
    PanResponder.create({
      onStartShouldSetPanResponder: () => true,
      onPanResponderGrant: () => {
        drag.current.from = live.current.start;
      },
      onPanResponderMove: (_event, gesture) => {
        const { track: width } = live.current;
        if (width <= 0) return;
        setStart(clampStart(drag.current.from + (gesture.dx / width) * duration, length, duration));
      },
    }),
  ).current;

  const done = async () => {
    setBusy(true);
    setFailed(false);
    try {
      player.pause();
      const uri = await trimFile(video.uri, start * 1000, (start + length) * 1000);
      onDone({
        ...video,
        uri,
        name: `${video.name.replace(/\.[^.]+$/, "")}.mp4`,
        contentType: "video/mp4",
        duration: length,
      });
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  };

  return (
    <SafeAreaView className="flex-1 bg-black" testID="trim-screen">
      <View className="gap-1 px-4 pt-2">
        <Text className="text-lg font-black text-white">{t("media.trimTitle")}</Text>
        <Text className="text-sm text-white/70">
          {t("media.trimLead", { limit: durationLimit(i18n, maxSeconds) })}
        </Text>
      </View>
      <View className="flex-1 py-3">
        <View pointerEvents="none" style={{ flex: 1 }}>
          <VideoView
            player={player}
            style={{ flex: 1 }}
            contentFit="contain"
            nativeControls={false}
          />
        </View>
      </View>
      <View className="gap-3 px-4 pb-4">
        <Text testID="trim-range" className="text-center font-bold text-white">
          {t("media.trimRange", {
            from: clock(start),
            to: clock(start + length),
            length: clock(length),
          })}
        </Text>
        <View
          testID="trim-track"
          className="h-14 justify-center rounded-xl bg-white/15"
          onLayout={(event) => setTrack(event.nativeEvent.layout.width)}
        >
          <View
            testID="trim-window"
            accessibilityRole="adjustable"
            accessibilityLabel={t("media.trimWindow")}
            accessibilityValue={{ text: `${clock(start)} – ${clock(start + length)}` }}
            {...pan.panHandlers}
            className="absolute h-14 rounded-xl border-4 border-sun bg-sun/25"
            style={{ left: track * (start / duration), width: windowWidth }}
          />
        </View>
        {failed && <ErrorNote>{t("media.trimFailed")}</ErrorNote>}
        <View className="flex-row justify-end gap-3">
          <Button variant="ghost" disabled={busy} onPress={onCancel}>
            {t("media.trimCancel")}
          </Button>
          <Button testID="trim-done" loading={busy} onPress={() => void done()}>
            {t("media.trimDone")}
          </Button>
        </View>
      </View>
    </SafeAreaView>
  );
}

/**
 * The trim screen as a step a composer awaits: `trim(video, max)` resolves
 * to the cut copy, or `null` when the person cancelled. `element` is the
 * modal to render.
 */
export function useTrimmer(): {
  trim(video: LocalMedia, maxSeconds: number): Promise<LocalMedia | null>;
  element: ReactNode;
} {
  const [job, setJob] = useState<{
    video: LocalMedia;
    maxSeconds: number;
    resolve(result: LocalMedia | null): void;
  } | null>(null);
  const finish = (result: LocalMedia | null) => {
    job?.resolve(result);
    setJob(null);
  };
  return {
    trim: (video, maxSeconds) => new Promise((resolve) => setJob({ video, maxSeconds, resolve })),
    element: (
      <Modal visible={job !== null} animationType="slide" onRequestClose={() => finish(null)}>
        {job && (
          <TrimScreen
            video={job.video}
            maxSeconds={job.maxSeconds}
            onDone={(trimmed) => finish(trimmed)}
            onCancel={() => finish(null)}
          />
        )}
      </Modal>
    ),
  };
}
