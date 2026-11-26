import { engagement, type shorts } from "@tokuly/api";
import { compactCount } from "@tokuly/core/engagement";
import {
  captionParts,
  type FeedEntry,
  imageUrl,
  type SlideLoad,
  StayMeter,
  shortSubject,
  soundTitle,
} from "@tokuly/core/shorts";
import { useEventListener } from "expo";
import { Image } from "expo-image";
import { router } from "expo-router";
import { useVideoPlayer, VideoView } from "expo-video";
import { Heart, MessageCircle, Music, Share2, Volume2, VolumeX } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { create } from "zustand";
import { Avatar } from "#/components/avatar";
import { useEngagement, useLike } from "#/features/engagement/data";
import { mediaUrl } from "#/lib/config";
import { errorMessage } from "#/lib/errors";
import { CommentsSheet } from "./comments-sheet";
import { reportStay, useShareShort } from "./data";

/** Two taps closer than this are a double tap. */
const DOUBLE_TAP_MS = 280;

/** Sound on or off, for every short of the feed (a choice that holds while scrolling). */
export const useShortsSound = create<{ muted: boolean; toggle(): void }>((set) => ({
  muted: false,
  toggle: () => set((state) => ({ muted: !state.muted })),
}));

function Action({
  testID,
  label,
  icon,
  count,
  onPress,
}: {
  testID: string;
  label: string;
  icon: React.ReactNode;
  count?: string;
  onPress(): void;
}) {
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      className="items-center gap-1"
    >
      <View className="rounded-full bg-black/30 p-2.5">{icon}</View>
      {count !== undefined && (
        <Text testID={`${testID}-count`} className="text-xs font-black text-white">
          {count}
        </Text>
      )}
    </Pressable>
  );
}

/**
 * One short of a swipe feed. The one on screen plays (looping, with the
 * feed's sound setting) and the next one loads ahead; the others show their
 * cover. A double tap likes; every stay is measured and reported.
 */
export function ShortSlide({
  entry,
  load,
  height,
  source,
  index,
}: {
  entry: FeedEntry;
  load: SlideLoad;
  height: number;
  source: shorts.PlaybackSource;
  index: number;
}) {
  const short = entry.short;
  const { t } = useTranslation("shorts");
  const live = load === "playing" || load === "preloading";
  const cover = mediaUrl(imageUrl(short.cover, 400));
  return (
    <View testID={`short-slide-${index}`} style={{ height }} className="bg-black">
      {live ? (
        <Player entry={entry} playing={load === "playing"} source={source} cover={cover} />
      ) : load === "cover" && cover ? (
        <Image source={cover} style={{ flex: 1 }} contentFit="cover" />
      ) : null}
      <Overlay entry={entry} />
      {entry.repeat && (
        <Text
          testID="short-repeat"
          className="absolute top-14 left-4 rounded-full bg-black/40 px-3 py-1 text-xs font-black text-white"
        >
          {t("reason.repeat")}
        </Text>
      )}
    </View>
  );
}

function Player({
  entry,
  playing,
  source,
  cover,
}: {
  entry: FeedEntry;
  playing: boolean;
  source: shorts.PlaybackSource;
  cover: string | undefined;
}) {
  const short = entry.short;
  const muted = useShortsSound((state) => state.muted);
  const uri = mediaUrl(short.playback?.hlsUrl || short.playback?.mp4Url);
  const player = useVideoPlayer(uri ? { uri } : null, (created) => {
    created.loop = true;
  });
  const [started, setStarted] = useState(false);
  const meter = useRef(new StayMeter());
  const subject = shortSubject(short);
  const like = useLike(subject);
  const { data: counts } = useEngagement(subject);
  const liked = counts
    ? counts.viewerReaction?.kind === engagement.ReactionKind.LIKE
    : (short.viewer?.liked ?? false);

  useEffect(() => {
    player.muted = muted;
  }, [player, muted]);

  // On screen it plays; off screen (loading ahead, or swiped past) it waits at the start.
  useEffect(() => {
    if (playing) player.play();
    else {
      player.pause();
      player.currentTime = 0;
    }
  }, [player, playing]);

  useEventListener(player, "playingChange", ({ isPlaying }) => {
    if (isPlaying) {
      setStarted(true);
      meter.current.play(Date.now());
    } else meter.current.pause(Date.now());
  });
  useEventListener(player, "playToEnd", () => meter.current.complete(Date.now()));

  // A stay ends when the short leaves the screen: it is reported, and the next one starts afresh.
  useEffect(() => {
    if (!playing) return;
    const stay = meter.current;
    return () => {
      reportStay(short.id, source, stay.read(Date.now()));
      meter.current = new StayMeter();
    };
  }, [playing, short.id, source]);

  // A double tap likes, a single one pauses or plays. Plain presses: the list around the
  // slide keeps the swipe to itself.
  const lastTap = useRef(0);
  const single = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(single.current), []);
  const onTap = () => {
    const now = Date.now();
    clearTimeout(single.current);
    if (now - lastTap.current < DOUBLE_TAP_MS) {
      lastTap.current = 0;
      if (!liked) void like(true);
      return;
    }
    lastTap.current = now;
    single.current = setTimeout(
      () => (player.playing ? player.pause() : player.play()),
      DOUBLE_TAP_MS,
    );
  };

  return (
    <Pressable testID="short-player" accessibilityRole="button" onPress={onTap} style={{ flex: 1 }}>
      {/* The native video view would take the touch: taps and the feed's swipe go around it. */}
      <View pointerEvents="none" style={{ flex: 1 }}>
        <VideoView player={player} style={{ flex: 1 }} contentFit="cover" nativeControls={false} />
      </View>
      {!started && cover ? (
        <Image source={cover} style={{ position: "absolute", inset: 0 }} contentFit="cover" />
      ) : null}
    </Pressable>
  );
}

function Overlay({ entry }: { entry: FeedEntry }) {
  const short = entry.short;
  const { t, i18n } = useTranslation("shorts");
  const { muted, toggle } = useShortsSound();
  const [comments, setComments] = useState(false);
  const subject = shortSubject(short);
  const { data: counts } = useEngagement(subject);
  const like = useLike(subject);
  const share = useShareShort(short);
  const liked = counts
    ? counts.viewerReaction?.kind === engagement.ReactionKind.LIKE
    : (short.viewer?.liked ?? false);
  const likeCount = counts ? counts.likeCount : Number(short.likeCount);
  const commentCount = counts ? counts.commentCount : Number(short.commentCount);
  const parts = captionParts(short.caption, short.hashtags, short.mentions);
  const count = (value: number) => compactCount(value, i18n.language);
  return (
    <>
      <View pointerEvents="box-none" className="absolute right-3 bottom-24 items-center gap-4">
        <Action
          testID="short-like"
          label={t("actions.like")}
          icon={
            <Heart
              color={liked ? "#ff5c9d" : "white"}
              fill={liked ? "#ff5c9d" : "transparent"}
              size={28}
            />
          }
          count={count(likeCount)}
          onPress={() => void like(!liked)}
        />
        <Action
          testID="short-comments"
          label={t("actions.comments")}
          icon={<MessageCircle color="white" size={28} />}
          count={count(commentCount)}
          onPress={() => setComments(true)}
        />
        <Action
          testID="short-share"
          label={t("actions.share")}
          icon={<Share2 color="white" size={28} />}
          count={count(Number(short.shareCount))}
          onPress={() =>
            void share().catch((error) => Alert.alert(errorMessage(error, { namespace: "shorts" })))
          }
        />
        <Action
          testID="short-sound"
          label={t(muted ? "player.unmute" : "player.mute")}
          icon={muted ? <VolumeX color="white" size={26} /> : <Volume2 color="white" size={26} />}
          onPress={toggle}
        />
      </View>
      <View pointerEvents="box-none" className="absolute right-20 bottom-6 left-4 gap-2">
        <View className="flex-row items-center gap-2">
          {entry.friend ? <Avatar user={entry.friend} size="xs" /> : null}
          <Text className="font-black text-white" numberOfLines={1}>
            {short.channel?.name}
          </Text>
        </View>
        {short.caption ? (
          <Text testID="short-caption" className="text-sm text-white" numberOfLines={3}>
            {parts.map((part, at) =>
              part.kind === "hashtag" ? (
                <Text
                  key={`${at}-${part.text}`}
                  className="font-black text-white"
                  onPress={() =>
                    router.push({ pathname: "/shorts/tag/[tag]", params: { tag: part.tag } })
                  }
                >
                  {part.text}
                </Text>
              ) : (
                <Text key={`${at}-${part.text}`}>{part.text}</Text>
              ),
            )}
          </Text>
        ) : null}
        {short.sound ? (
          <Pressable
            testID="short-sound-page"
            accessibilityRole="link"
            onPress={() =>
              router.push({ pathname: "/shorts/sound/[id]", params: { id: short.sound!.id } })
            }
            className="flex-row items-center gap-1.5"
          >
            <Music color="white" size={14} />
            <Text className="text-xs font-bold text-white" numberOfLines={1}>
              {soundTitle(short.sound, (name) => t("sound.original", { name }))}
            </Text>
          </Pressable>
        ) : null}
      </View>
      <CommentsSheet short={short} visible={comments} onClose={() => setComments(false)} />
    </>
  );
}
