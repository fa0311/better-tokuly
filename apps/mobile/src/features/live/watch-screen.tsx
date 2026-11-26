import { useQueryClient } from "@tanstack/react-query";
import { common, engagement, live, toMillis } from "@tokuly/api";
import type { Subject } from "@tokuly/core/engagement";
import { compactCount } from "@tokuly/core/engagement";
import { refreshStream } from "@tokuly/core/live";
import { Image } from "expo-image";
import { router } from "expo-router";
import {
  Bell,
  BellRing,
  ChevronLeft,
  Heart,
  MoreHorizontal,
  Share2,
  Swords,
} from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Alert,
  AppState,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { ActionSheet, type SheetAction } from "#/components/action-sheet";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { useEngagement, useLike } from "#/features/engagement/data";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { afterSheet, ReportSheet } from "./actions";
import { LiveChat } from "./chat-panel";
import { shareStream, streamClient, useStream } from "./data";
import {
  ChannelFace,
  Countdown,
  formatMoment,
  LiveMark,
  StreamCard,
  thumbnailOf,
  ViewerCount,
} from "./parts";
import { LivePlayer } from "./player";
import { useLiveRoom, useWatchingReport } from "./room";

const { StreamStatus, ArchiveStatus } = live;

const openWeb = (path: string) => router.push(webScreenHref(path) as never);

/** Whether the app is in front: a stream in the background is not watched (nor reported as watched). */
function useInFront(): boolean {
  const [inFront, setInFront] = useState(AppState.currentState !== "background");
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setInFront(state !== "background"),
    );
    return () => subscription.remove();
  }, []);
  return inFront;
}

/** `/live/<id>`: a stream with its chat, whether it is scheduled, on air or over. */
export function WatchLiveScreen({ streamId }: { streamId: string }) {
  const { t } = useTranslation("live");
  const query = useStream(streamId);
  if (query.isPending)
    return (
      <SafeAreaView className="flex-1 justify-center bg-bg">
        <Spinner />
      </SafeAreaView>
    );
  if (!query.data)
    return (
      <SafeAreaView testID="live-not-found" className="flex-1 justify-center gap-4 bg-bg px-6">
        <EmptyState emoji="📡" title={t("notFound.title")} body={t("notFound.body")} />
        <Button variant="secondary" onPress={() => router.replace("/live")}>
          {t("notFound.action")}
        </Button>
      </SafeAreaView>
    );
  return <StreamRoom key={query.data.summary?.id} stream={query.data} />;
}

function StreamRoom({ stream }: { stream: live.Stream }) {
  const { t } = useTranslation("live");
  const queryClient = useQueryClient();
  const summary = stream.summary;
  const streamId = summary?.id ?? "";
  const status = summary?.status ?? StreamStatus.UNSPECIFIED;
  const inFront = useInFront();
  const raid = useLiveRoom(stream);
  useWatchingReport(stream, inFront);

  // The recording becomes a video a little after the end.
  const archivePending =
    status === StreamStatus.ENDED &&
    (stream.archiveStatus === ArchiveStatus.PENDING ||
      stream.archiveStatus === ArchiveStatus.PROCESSING);
  useEffect(() => {
    if (!archivePending) return;
    const timer = setInterval(() => refreshStream(queryClient, streamId), 6000);
    return () => clearInterval(timer);
  }, [archivePending, queryClient, streamId]);

  if (!summary) return null;
  const stage =
    status === StreamStatus.LIVE && stream.playback && inFront ? (
      <LivePlayer
        playback={stream.playback}
        lowLatency={summary.lowLatency}
        poster={thumbnailOf(summary, 640)}
        onSessionLost={() => refreshStream(queryClient, streamId)}
      />
    ) : status === StreamStatus.ENDED ? (
      <EndedStage stream={stream} />
    ) : (
      <UpcomingStage stream={stream} />
    );

  return (
    <SafeAreaView edges={["top", "bottom"]} className="flex-1 bg-black" testID="live-room">
      <KeyboardAvoidingView
        className="flex-1 bg-surface"
        behavior={Platform.OS === "ios" ? "padding" : undefined}
      >
        <View className="bg-black">
          {stage}
          <Pressable
            testID="live-back"
            accessibilityRole="button"
            accessibilityLabel={t("back")}
            onPress={() => (router.canGoBack() ? router.back() : router.replace("/live"))}
            className="absolute top-2 left-2 rounded-full bg-black/55 p-1.5"
          >
            <ChevronLeft size={24} color="#ffffff" />
          </Pressable>
        </View>
        {(raid || stream.raidTargetStreamId) && (
          <RaidBanner
            target={raid?.target}
            targetId={raid?.target?.shortId ?? stream.raidTargetStreamId}
            automatic={raid !== null}
          />
        )}
        <StreamInfo stream={stream} />
        <LiveChat stream={stream} />
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

/** A stream that has not started: its picture, when it starts, and "tell me when it does". */
function UpcomingStage({ stream }: { stream: live.Stream }) {
  const { t, i18n } = useTranslation("live");
  const queryClient = useQueryClient();
  const summary = stream.summary;
  const streamId = summary?.id ?? "";
  const scheduledMs = summary?.scheduledAt ? toMillis(summary.scheduledAt) : undefined;
  const poster = summary ? thumbnailOf(summary, 640) : undefined;
  const [reminder, setReminder] = useState(Boolean(stream.viewer?.reminderSet));
  const [busy, setBusy] = useState(false);
  const remind = async (enabled: boolean) => {
    const client = streamClient();
    setReminder(enabled);
    setBusy(true);
    try {
      const answer = await client.setReminder({ streamId, enabled });
      setReminder(answer.enabled);
      Alert.alert(t(answer.enabled ? "upcoming.reminderOn" : "upcoming.reminderOff"));
      refreshStream(queryClient, streamId);
    } catch (error) {
      setReminder(!enabled);
      Alert.alert(errorMessage(error, { namespace: "live" }));
    } finally {
      setBusy(false);
    }
  };
  return (
    <View
      testID="upcoming-stage"
      className="aspect-video w-full items-center justify-center bg-[#17122b]"
    >
      {poster && (
        <Image
          source={poster}
          style={{ position: "absolute", inset: 0, opacity: 0.45 }}
          contentFit="cover"
        />
      )}
      <View className="items-center gap-2 p-4">
        <Text className="rounded-full bg-white/15 px-3 py-1 text-xs font-black text-white">
          {t("upcoming.badge")}
        </Text>
        {scheduledMs !== undefined ? (
          <>
            <Countdown targetMs={scheduledMs} className="text-3xl font-black text-white" />
            <Text className="text-sm font-bold text-white/85">
              {t("upcoming.startsAt", { time: formatMoment(scheduledMs, i18n.language) })}
            </Text>
          </>
        ) : (
          <Text className="text-xl font-black text-white">{t("upcoming.unscheduled")}</Text>
        )}
        {stream.viewer?.isOwner ? (
          <Button
            variant="secondary"
            size="sm"
            onPress={() => openWeb(`/studio/live/${summary?.shortId ?? ""}`)}
          >
            {t("app.studio")}
          </Button>
        ) : (
          <Button
            testID="live-remind"
            variant={reminder ? "lime" : "secondary"}
            size="sm"
            disabled={busy}
            icon={
              reminder ? <BellRing size={16} color="#17122b" /> : <Bell size={16} color="#17122b" />
            }
            onPress={() => void remind(!reminder)}
          >
            {t(reminder ? "upcoming.reminderSet" : "upcoming.remind")}
          </Button>
        )}
      </View>
    </View>
  );
}

/** A stream that is over: the archive (a video, on the web screen) when there is one. */
function EndedStage({ stream }: { stream: live.Stream }) {
  const { t } = useTranslation("live");
  const summary = stream.summary;
  const poster = summary ? thumbnailOf(summary, 640) : undefined;
  const archive = stream.archiveStatus;
  return (
    <View
      testID="ended-stage"
      className="aspect-video w-full items-center justify-center bg-[#17122b]"
    >
      {poster && (
        <Image
          source={poster}
          style={{ position: "absolute", inset: 0, opacity: 0.3 }}
          contentFit="cover"
        />
      )}
      <View className="items-center gap-2 p-4">
        <Text className="text-lg font-black text-white">{t("ended.title")}</Text>
        {stream.archiveVideoId ? (
          <Button size="sm" onPress={() => openWeb(`/watch/${stream.archiveVideoId}`)}>
            {t("app.archive")}
          </Button>
        ) : archive === ArchiveStatus.PENDING || archive === ArchiveStatus.PROCESSING ? (
          <Text className="text-sm font-bold text-white/85">{t("archive.processing")}</Text>
        ) : (
          <Text className="text-sm font-bold text-white/85">{t("ended.noArchive")}</Text>
        )}
      </View>
    </View>
  );
}

/** The end of a stream that sent its viewers on: where to, and a ride there unless they stay. */
function RaidBanner({
  target,
  targetId,
  automatic,
}: {
  target: live.StreamSummary | undefined;
  targetId: string;
  /** The raid just happened in front of the viewer: they are taken along unless they stay. */
  automatic: boolean;
}) {
  const { t } = useTranslation("live");
  const loaded = useStream(target ? "" : targetId);
  const summary = target ?? loaded.data?.summary;
  const [left, setLeft] = useState<number | null>(automatic ? 8 : null);
  useEffect(() => {
    if (left === null) return;
    if (left <= 0) {
      router.replace(`/live/${targetId}`);
      return;
    }
    const timer = setTimeout(() => setLeft(left - 1), 1000);
    return () => clearTimeout(timer);
  }, [left, targetId]);
  if (!summary) return null;
  return (
    <View testID="raid-banner" className="mx-3 mt-2 gap-2 rounded-2xl bg-brand-soft p-3">
      <View className="flex-row items-center gap-2">
        <Swords size={16} color="#5f3df0" />
        <Text className="text-sm font-black text-brand-strong">
          {t("raid.banner", { name: summary.channel?.name ?? "" })}
        </Text>
      </View>
      <StreamCard summary={summary} width={220} />
      {left !== null && (
        <View className="flex-row items-center justify-between">
          <Text className="text-sm font-bold text-ink-soft">
            {t("raid.moving", { count: left })}
          </Text>
          <Button variant="ghost" size="sm" onPress={() => setLeft(null)}>
            {t("raid.stay")}
          </Button>
        </View>
      )}
    </View>
  );
}

/** A like with the stream's count (pushed live), and a heart that pops when someone else likes. */
function LikeButton({ stream }: { stream: live.Stream }) {
  const { t, i18n } = useTranslation("live");
  const { colors } = useTheme();
  const subject: Subject = { type: common.SubjectType.LIVE_STREAM, id: stream.summary?.id ?? "" };
  const like = useLike(subject);
  const { data: counts } = useEngagement(subject);
  const liked = counts?.viewerReaction?.kind === engagement.ReactionKind.LIKE;
  const count = stream.summary?.likeCount ?? 0n;
  return (
    <Pressable
      testID="live-like"
      accessibilityRole="button"
      accessibilityLabel={t(liked ? "app.unlike" : "app.like")}
      accessibilityState={{ selected: liked }}
      onPress={() => void like(!liked)}
      className={`h-9 flex-row items-center gap-1.5 rounded-full px-3 ${liked ? "bg-pink/15" : "bg-surface-2"}`}
    >
      <Heart
        size={18}
        color={liked ? colors.pink : colors.ink}
        fill={liked ? colors.pink : "transparent"}
      />
      <Text testID="live-like-count" className="text-sm font-black text-ink">
        {compactCount(Number(count), i18n.language)}
      </Text>
    </Pressable>
  );
}

/** The line under the picture: who, what, how many, and what can be done. */
function StreamInfo({ stream }: { stream: live.Stream }) {
  const { t } = useTranslation("live");
  const { colors } = useTheme();
  const [menu, setMenu] = useState(false);
  const [reporting, setReporting] = useState<Subject | null>(null);
  const summary = stream.summary;
  if (!summary) return null;
  const channel = summary.channel;
  const onAir = summary.status === StreamStatus.LIVE;
  const owner = Boolean(stream.viewer?.isOwner);
  const actions: SheetAction[] = [
    ...(owner
      ? [
          {
            key: "studio",
            label: t("app.studio"),
            onPress: () => openWeb(`/studio/live/${summary.shortId}`),
          },
        ]
      : []),
    ...(stream.viewer?.isModerator
      ? [
          {
            key: "moderation",
            label: t("app.moderation"),
            onPress: () => openWeb("/studio/live/moderation"),
          },
        ]
      : []),
    ...(onAir && stream.dvrEnabled
      ? [
          {
            key: "clip",
            label: t("app.clip"),
            onPress: () => openWeb(`/live/clips/new?stream=${summary.shortId}`),
          },
        ]
      : []),
    { key: "web", label: t("app.openOnWeb"), onPress: () => openWeb(`/live/${summary.shortId}`) },
    ...(owner
      ? []
      : [
          {
            key: "report",
            label: t("app.report.stream"),
            destructive: true,
            onPress: () =>
              afterSheet(() =>
                setReporting({ type: common.SubjectType.LIVE_STREAM, id: summary.id }),
              ),
          },
        ]),
  ];
  return (
    <View className="gap-2 border-b-2 border-line px-3 py-2">
      <View className="flex-row items-center gap-2.5">
        {channel && <ChannelFace channel={channel} size="sm" />}
        <View className="min-w-0 flex-1">
          <Text testID="live-title" numberOfLines={1} className="text-sm font-black text-ink">
            {summary.title}
          </Text>
          <View className="flex-row items-center gap-2">
            {onAir && <LiveMark />}
            {onAir ? (
              <ViewerCount count={summary.viewerCount} />
            ) : (
              <Text className="text-xs font-bold text-ink-soft">
                {t(summary.status === StreamStatus.ENDED ? "status.ended" : "status.upcoming")}
              </Text>
            )}
            {channel && (
              <Text numberOfLines={1} className="shrink text-xs font-bold text-ink-faint">
                {channel.name}
              </Text>
            )}
          </View>
        </View>
      </View>
      <View className="flex-row items-center gap-1.5">
        <LikeButton stream={stream} />
        <Pressable
          testID="live-share"
          accessibilityRole="button"
          accessibilityLabel={t("app.share")}
          onPress={() => void shareStream(summary).catch(() => {})}
          className="h-9 flex-row items-center gap-1.5 rounded-full bg-surface-2 px-3"
        >
          <Share2 size={18} color={colors.ink} />
        </Pressable>
        <View className="flex-1" />
        <Pressable
          testID="live-more"
          accessibilityRole="button"
          accessibilityLabel={t("app.more")}
          onPress={() => setMenu(true)}
          className="h-9 w-9 items-center justify-center rounded-full"
        >
          <MoreHorizontal size={20} color={colors["ink-soft"]} />
        </Pressable>
      </View>
      <ActionSheet
        visible={menu}
        title={summary.title}
        actions={actions}
        onClose={() => setMenu(false)}
      />
      <ReportSheet subject={reporting} onClose={() => setReporting(null)} />
    </View>
  );
}
