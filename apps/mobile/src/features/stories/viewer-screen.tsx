import { useQueryClient } from "@tanstack/react-query";
import { type common, media, type notes, toMillis } from "@tokuly/api";
import { compactAge } from "@tokuly/core/format";
import { reelStart, removeStory, type StorySegment, storySegments } from "@tokuly/core/stories";
import { useEvent, useEventListener } from "expo";
import { Image } from "expo-image";
import { router } from "expo-router";
import { useVideoPlayer, VideoView } from "expo-video";
import { Ellipsis, Eye, Send, Volume2, VolumeX, X } from "lucide-react-native";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Alert,
  AppState,
  KeyboardAvoidingView,
  Pressable,
  Text,
  TextInput,
  useWindowDimensions,
  View,
} from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, { useAnimatedStyle, useSharedValue } from "react-native-reanimated";
import { SafeAreaView } from "react-native-safe-area-context";
import { ActionSheet } from "#/components/action-sheet";
import { Avatar } from "#/components/avatar";
import { Spinner } from "#/components/page";
import { useMe } from "#/features/session/store";
import { mediaUrl } from "#/lib/config";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { storyClient, useStoryTray, useUserStories } from "./data";
import { replyInChat } from "./reply";
import { seenMarker } from "./seen";
import { ViewersSheet } from "./viewers-sheet";

/** How long a photo stays before the next one. */
const PHOTO_MS = 5000;
const TICK_MS = 50;
const SWIPE_REEL = 56;
const SWIPE_CLOSE = 90;
const QUICK_REACTIONS = ["❤️", "😂", "😮", "😢", "👏", "🔥"];

interface Reel {
  user?: common.UserSummary;
  stories: notes.Story[];
}

const isVideo = (segment: StorySegment) => segment.item.media?.kind === media.AssetKind.VIDEO;

function videoSource(segment: StorySegment): string | undefined {
  const video = segment.item.media?.video;
  return mediaUrl(video?.hlsUrl || video?.mp4Url || segment.item.media?.url);
}

function pictureOf(segment: StorySegment): string | undefined {
  const attachment = segment.item.media;
  const image = attachment?.video?.poster ?? attachment?.image;
  return mediaUrl(image?.url || attachment?.url);
}

/**
 * Stories full screen, opened on one person's reel: the tray's reels play on
 * after it (a person's ring elsewhere plays only theirs). The reels are
 * taken as they were when it opened, so nothing moves under the viewer.
 */
export function StoryViewerScreen({ userId }: { userId: string }) {
  const tray = useStoryTray();
  const inTray = tray.data?.some((reel) => reel.user?.id === userId) ?? false;
  const single = useUserStories(userId, !tray.isPending && !inTray);
  const snapshot = useRef<{ reels: Reel[]; start: number } | null>(null);
  if (!snapshot.current) {
    if (inTray && tray.data) {
      const reels = tray.data.filter((reel) => reel.stories.some((story) => story.items.length));
      snapshot.current = {
        reels,
        start: Math.max(
          0,
          reels.findIndex((reel) => reel.user?.id === userId),
        ),
      };
    } else if (single.data?.length)
      snapshot.current = {
        reels: [{ user: single.data[0]?.owner, stories: single.data }],
        start: 0,
      };
  }
  const missing = !snapshot.current && !tray.isPending && !single.isPending;
  useEffect(() => {
    // Nothing to play any more (expired, deleted, or no longer a friend's).
    if (missing) router.back();
  }, [missing]);
  if (!snapshot.current) return missing ? null : <Spinner />;
  return <Player reels={snapshot.current.reels} startReel={snapshot.current.start} />;
}

function Player({ reels, startReel }: { reels: Reel[]; startReel: number }) {
  const { t } = useTranslation("stories");
  const { t: tm } = useTranslation("mobile");
  const queryClient = useQueryClient();
  const me = useMe();
  const { width } = useWindowDimensions();
  const [position, setPosition] = useState(() => ({
    reel: startReel,
    segment: reelStart(reels[startReel]?.stories ?? []),
  }));
  const [ready, setReady] = useState(false);
  const [holding, setHolding] = useState(false);
  const [typing, setTyping] = useState(false);
  const [sheet, setSheet] = useState<"viewers" | "menu" | null>(null);
  const [inFront, setInFront] = useState(AppState.currentState !== "background");
  const [muted, setMuted] = useState(false);
  const [removed, setRemoved] = useState<ReadonlySet<string>>(new Set());
  /** The viewer's reactions made here, by story; `""` = taken back. */
  const [reacted, setReacted] = useState<Record<string, string>>({});
  const [reply, setReply] = useState("");
  const [replied, setReplied] = useState(false);
  const mark = useMemo(() => seenMarker(queryClient), [queryClient]);
  const progress = useSharedValue(0);

  const reel = reels[position.reel]!;
  const segments = storySegments(reel.stories).filter((segment) => !removed.has(segment.story.id));
  const segment = segments[Math.min(position.segment, segments.length - 1)];
  const mine = Boolean(me?.user && reel.user?.id === me.user.id);
  const paused = holding || typing || sheet !== null || !inFront || !ready;

  const player = useVideoPlayer(null, (created) => {
    created.loop = false;
  });
  const status = useEvent(player, "statusChange", { status: player.status });

  const close = () => (router.canGoBack() ? router.back() : router.replace("/chats"));
  const goReel = (delta: number) => {
    const next = position.reel + delta;
    if (next < 0) return;
    if (next >= reels.length) return close();
    setReady(false);
    setPosition({ reel: next, segment: reelStart(reels[next]!.stories) });
  };
  const next = () => {
    if (position.segment < segments.length - 1) {
      setReady(false);
      setPosition({ ...position, segment: position.segment + 1 });
    } else goReel(1);
  };
  const previous = () => {
    if (position.segment > 0) {
      setReady(false);
      setPosition({ ...position, segment: position.segment - 1 });
    } else if (position.reel > 0) goReel(-1);
    else if (segment && isVideo(segment)) player.currentTime = 0;
  };
  // The latest of these for the timer, which outlives a render.
  const live = useRef({ next, paused, video: false });

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setInFront(state === "active"),
    );
    return () => subscription.remove();
  }, []);

  // A reel that lost its last story (deleted from here) has nothing left to show.
  const empty = segments.length === 0;
  useEffect(() => {
    if (empty) live.current.next();
  }, [empty]);

  // A video loads into the one player; a photo is ready once its picture is up.
  const key = segment?.key;
  const video = segment ? isVideo(segment) : false;
  const source = segment && video ? videoSource(segment) : undefined;
  live.current = { next, paused, video };
  useEffect(() => {
    if (source) void player.replaceAsync({ uri: source });
    else player.pause();
  }, [source, player]);
  useEffect(() => {
    if (video && status.status === "readyToPlay") setReady(true);
  }, [video, status.status]);
  // Only a video on screen moves on by ending: an empty player (a photo is up) reports "ended" too.
  useEventListener(player, "playToEnd", () => {
    if (live.current.video) live.current.next();
  });

  useEffect(() => {
    player.muted = muted;
  }, [player, muted]);
  useEffect(() => {
    if (!video) return;
    if (paused && ready) player.pause();
    else if (!paused) player.play();
  }, [video, paused, ready, player]);

  // The bar of the piece on screen: a photo runs on a clock, a video on its own time.
  useEffect(() => {
    if (!key) return;
    progress.value = 0;
    let elapsed = 0;
    const timer = setInterval(() => {
      if (video) {
        if (player.duration > 0) progress.value = player.currentTime / player.duration;
        return;
      }
      if (live.current.paused) return;
      elapsed += TICK_MS;
      progress.value = elapsed / PHOTO_MS;
      if (elapsed >= PHOTO_MS) live.current.next();
    }, TICK_MS);
    return () => clearInterval(timer);
  }, [key, video, player, progress]);

  // Watching counts once the story is really shown.
  const story = segment?.story;
  useEffect(() => {
    if (story && ready && inFront) mark(story, mine);
  }, [story, ready, inFront, mine, mark]);

  const bar = useAnimatedStyle(() => ({ width: `${Math.min(1, progress.value) * 100}%` }));

  if (!segment || !story) return null;

  const myReaction = reacted[story.id] ?? story.myReaction;
  const react = async (emoji: string) => {
    const taking = myReaction === emoji;
    const stories = storyClient();
    setReacted({ ...reacted, [story.id]: taking ? "" : emoji });
    try {
      if (taking) await stories.removeStoryReaction({ storyId: story.id });
      else await stories.reactToStory({ storyId: story.id, emoji });
    } catch (error) {
      setReacted({ ...reacted, [story.id]: myReaction });
      Alert.alert(errorMessage(error, { namespace: "stories" }));
    }
  };
  const sendReply = async () => {
    const text = reply.trim();
    if (!text || !reel.user) return;
    try {
      await replyInChat(queryClient, reel.user.id, story, text);
      setReply("");
      setReplied(true);
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "chat" }));
    }
  };
  const deleteStory = async () => {
    try {
      await storyClient().deleteStory({ storyId: story.id });
      setRemoved(new Set(removed).add(story.id));
      removeStory(queryClient, story.id);
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "stories" }));
    }
  };

  const tap = Gesture.Tap()
    .runOnJS(true)
    .maxDuration(250)
    .onEnd((event) => (event.x < width * 0.3 ? previous() : next()));
  const hold = Gesture.LongPress()
    .runOnJS(true)
    .minDuration(220)
    .onStart(() => setHolding(true))
    .onFinalize(() => setHolding(false));
  const swipe = Gesture.Pan()
    .runOnJS(true)
    .minDistance(14)
    .onEnd((event) => {
      if (Math.abs(event.translationX) > Math.abs(event.translationY)) {
        if (event.translationX <= -SWIPE_REEL) goReel(1);
        else if (event.translationX >= SWIPE_REEL) goReel(-1);
      } else if (event.translationY >= SWIPE_CLOSE) close();
    });

  const posted = toMillis(story.createdAt);
  return (
    <View className="flex-1 bg-black" testID="story-viewer">
      <GestureDetector gesture={Gesture.Race(swipe, hold, tap)}>
        <View className="absolute inset-0">
          {video ? (
            <View pointerEvents="none" style={{ flex: 1 }}>
              <VideoView
                player={player}
                style={{ flex: 1 }}
                contentFit="contain"
                nativeControls={false}
              />
            </View>
          ) : (
            <Image
              testID="story-picture"
              source={pictureOf(segment)}
              style={{ flex: 1 }}
              contentFit="contain"
              onLoad={() => setReady(true)}
              onError={() => setReady(true)}
            />
          )}
        </View>
      </GestureDetector>
      <SafeAreaView className="flex-1 justify-between" pointerEvents="box-none">
        <View className="gap-2 px-3 pt-2" pointerEvents="box-none">
          <View className="flex-row gap-1">
            {segments.map((piece, index) => (
              <View key={piece.key} className="h-1 flex-1 overflow-hidden rounded-full bg-white/30">
                {index < position.segment ? (
                  <View className="h-full w-full bg-white" />
                ) : index === position.segment ? (
                  <Animated.View className="h-full bg-white" style={bar} />
                ) : null}
              </View>
            ))}
          </View>
          <View className="flex-row items-center gap-2">
            <Avatar user={reel.user} size="sm" />
            <Text className="font-black text-white" numberOfLines={1}>
              {mine ? t("tray.mine") : reel.user?.displayName}
            </Text>
            {posted > 0 && (
              <Text className="text-sm text-white/70">{compactAge(i18n, posted)}</Text>
            )}
            <View className="ml-auto flex-row items-center gap-1">
              {video && (
                <Pressable
                  testID="story-mute"
                  accessibilityRole="button"
                  accessibilityLabel={t(muted ? "viewer.unmute" : "viewer.mute")}
                  onPress={() => setMuted(!muted)}
                  className="p-2"
                >
                  {muted ? (
                    <VolumeX color="white" size={22} />
                  ) : (
                    <Volume2 color="white" size={22} />
                  )}
                </Pressable>
              )}
              {mine && (
                <Pressable
                  testID="story-menu"
                  accessibilityRole="button"
                  accessibilityLabel={t("viewer.menu")}
                  onPress={() => setSheet("menu")}
                  className="p-2"
                >
                  <Ellipsis color="white" size={22} />
                </Pressable>
              )}
              <Pressable
                testID="story-close"
                accessibilityRole="button"
                accessibilityLabel={t("viewer.close")}
                onPress={close}
                className="p-2"
              >
                <X color="white" size={24} />
              </Pressable>
            </View>
          </View>
          {story.title ? (
            <Text className="text-base font-bold text-white">{story.title}</Text>
          ) : null}
        </View>
        <KeyboardAvoidingView behavior="padding" pointerEvents="box-none">
          <View className="gap-2 px-3 pb-3" pointerEvents="box-none">
            {segment.item.caption ? (
              <Text className="self-center rounded-xl bg-black/50 px-3 py-1.5 text-center text-base text-white">
                {segment.item.caption}
              </Text>
            ) : null}
            {mine ? (
              <Pressable
                testID="story-viewers"
                accessibilityRole="button"
                onPress={() => setSheet("viewers")}
                className="flex-row items-center gap-2 self-start rounded-full bg-black/50 px-4 py-2"
              >
                <Eye color="white" size={18} />
                <Text className="font-black text-white">
                  {t("viewer.views", { count: story.viewCount })}
                  {story.reactionCount > 0
                    ? ` · ${t("viewer.reactions", { count: story.reactionCount })}`
                    : ""}
                </Text>
              </Pressable>
            ) : (
              <>
                <View className="flex-row justify-around">
                  {QUICK_REACTIONS.map((emoji, index) => (
                    <Pressable
                      key={emoji}
                      testID={`story-react-${index}`}
                      accessibilityRole="button"
                      accessibilityLabel={`${t("viewer.react")} ${emoji}`}
                      accessibilityState={{ selected: myReaction === emoji }}
                      onPress={() => void react(emoji)}
                      className={`rounded-full p-2 ${myReaction === emoji ? "bg-white/40" : ""}`}
                    >
                      <Text className="text-2xl">{emoji}</Text>
                    </Pressable>
                  ))}
                </View>
                <View className="flex-row items-center gap-2">
                  <TextInput
                    testID="story-reply-input"
                    value={reply}
                    onChangeText={(text) => {
                      setReply(text);
                      setReplied(false);
                    }}
                    onFocus={() => setTyping(true)}
                    onBlur={() => setTyping(false)}
                    placeholder={tm("stories.replyPlaceholder", {
                      name: reel.user?.displayName ?? "",
                    })}
                    placeholderTextColor="rgba(255,255,255,0.6)"
                    className="flex-1 rounded-full border border-white/50 px-4 py-2.5 text-base text-white"
                    returnKeyType="send"
                    onSubmitEditing={() => void sendReply()}
                  />
                  <Pressable
                    testID="story-reply-send"
                    accessibilityRole="button"
                    accessibilityLabel={tm("stories.replySend")}
                    disabled={!reply.trim()}
                    onPress={() => void sendReply()}
                    className="p-2"
                  >
                    <Send color="white" size={24} />
                  </Pressable>
                </View>
                {replied && (
                  <Text className="text-center text-sm font-bold text-white">
                    {tm("stories.replied")}
                  </Text>
                )}
              </>
            )}
          </View>
        </KeyboardAvoidingView>
      </SafeAreaView>
      {mine && (
        <ViewersSheet
          storyId={story.id}
          visible={sheet === "viewers"}
          onClose={() => setSheet(null)}
        />
      )}
      <ActionSheet
        visible={sheet === "menu"}
        onClose={() => setSheet(null)}
        actions={[
          {
            key: "delete",
            label: t("viewer.delete"),
            destructive: true,
            onPress: () =>
              Alert.alert(t("viewer.deleteTitle"), t("viewer.deleteBody"), [
                { text: t("create.cancel"), style: "cancel" },
                {
                  text: t("viewer.delete"),
                  style: "destructive",
                  onPress: () => void deleteStory(),
                },
              ]),
          },
        ]}
      />
    </View>
  );
}
