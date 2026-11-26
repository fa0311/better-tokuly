import { useQueryClient } from "@tanstack/react-query";
import { common, community, engagement } from "@tokuly/api";
import {
  communityKeys,
  imageUrl,
  isMember,
  patchThread,
  pseudonymName,
  splitMentions,
  timeoutUntilMs,
  upsertReply,
} from "@tokuly/core/community";
import { Image } from "expo-image";
import { router, Stack } from "expo-router";
import { Bell, Heart, SendHorizontal, X } from "lucide-react-native";
import { type ComponentRef, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Alert,
  FlatList,
  KeyboardAvoidingView,
  Pressable,
  Text,
  TextInput,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { EmptyState, Spinner } from "#/components/page";
import { useEngagement, useLike } from "#/features/engagement/data";
import { mediaUrl } from "#/lib/config";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { boardClient, itemsOf, NS, useCommunity, useReplies, useThread } from "./data";
import { Byline } from "./parts";
import { PollCard } from "./poll";

const complain = (error: unknown) => Alert.alert(errorMessage(error, { namespace: NS }));

/** A post's text, the members it mentions in the brand colour (they open their profiles). */
function Body({ text, mentions }: { text: string; mentions: community.Mention[] }) {
  if (text === "") return null;
  return (
    <Text selectable className="text-base leading-6 text-ink">
      {splitMentions(text, mentions).map((part, index) =>
        part.handle ? (
          <Text
            key={index}
            className="font-bold text-brand"
            onPress={() => router.push(`/u/${part.handle}`)}
          >
            {part.text}
          </Text>
        ) : (
          <Text key={index}>{part.text}</Text>
        ),
      )}
    </Text>
  );
}

function Images({ images }: { images: common.Image[] }) {
  if (images.length === 0) return null;
  return (
    <View className="gap-2">
      {images.map((image) => {
        const url = mediaUrl(imageUrl(image, 360));
        return url ? (
          <Image
            key={image.assetId || url}
            source={url}
            contentFit="cover"
            style={{
              width: "100%",
              aspectRatio: image.width && image.height ? image.width / image.height : 4 / 3,
              borderRadius: 16,
            }}
          />
        ) : null;
      })}
    </View>
  );
}

/** `/g/<handle>/t/<id>`: a thread, its replies oldest first, and writing one. */
export function ThreadScreen({ handle, threadId }: { handle: string; threadId: string }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const c = useCommunity(handle).data;
  const query = useThread(threadId);
  const thread = query.data;
  const replies = useReplies(thread?.id ?? "", Boolean(thread));
  const [replyTo, setReplyTo] = useState<community.Reply | null>(null);
  const keyboardArea = useRef<ComponentRef<typeof View>>(null);
  const [top, setTop] = useState(0);
  const member = isMember(c ?? undefined);
  const threadUuid = thread?.id;
  const unread = thread?.viewer?.unread ?? false;

  // Opening a thread reads it.
  useEffect(() => {
    if (!threadUuid || !member || !unread) return;
    boardClient()
      .markRead({ target: { case: "threadId", value: threadUuid } })
      .then(() => {
        patchThread(queryClient, threadUuid, (each) =>
          each.viewer
            ? { ...each, viewer: { ...each.viewer, unread: false, unreadReplyCount: 0 } }
            : each,
        );
        void queryClient.invalidateQueries({ queryKey: communityKeys.communities });
        void queryClient.invalidateQueries({ queryKey: communityKeys.my });
      })
      .catch(() => {});
  }, [threadUuid, member, unread, queryClient]);

  if (query.isPending) return <Spinner />;
  if (!thread)
    return (
      <View className="flex-1 bg-bg" testID="thread-not-found">
        <EmptyState emoji="🫥" title={t("thread.gone")} body={t("thread.goneBody")} />
      </View>
    );

  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="thread-screen">
      {/* Where this screen starts on the window: below the header. */}
      <View
        ref={keyboardArea}
        onLayout={() => keyboardArea.current?.measureInWindow((_x, y) => setTop(y))}
      />
      <Stack.Screen options={{ title: c?.name ?? "" }} />
      <KeyboardAvoidingView
        className="flex-1"
        // Android draws edge to edge: the window is not resized for the keyboard, so both pad,
        // by as much as the header above this screen takes.
        behavior="padding"
        keyboardVerticalOffset={top}
      >
        <FlatList
          data={itemsOf(replies.data)}
          keyExtractor={(reply) => reply.id}
          renderItem={({ item }) => <ReplyRow reply={item} onReply={() => setReplyTo(item)} />}
          refreshing={query.isRefetching}
          onRefresh={() => {
            void query.refetch();
            void replies.refetch();
          }}
          onEndReached={() => {
            if (replies.hasNextPage && !replies.isFetchingNextPage) void replies.fetchNextPage();
          }}
          ListHeaderComponent={<ThreadHead thread={thread} member={member} />}
          ListEmptyComponent={
            replies.isPending ? (
              <Spinner />
            ) : (
              <Text className="px-4 py-8 text-center text-sm text-ink-soft">
                {t("reply.empty")}
              </Text>
            )
          }
          ListFooterComponent={
            <View className="h-6">{replies.isFetchingNextPage && <Spinner />}</View>
          }
        />
        <ReplyComposer
          thread={thread}
          member={member}
          timedOut={timeoutUntilMs(c?.viewer, Date.now()) > 0}
          replyTo={replyTo}
          onClearReplyTo={() => setReplyTo(null)}
        />
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

/** The thread itself: who wrote it, its text, pictures and poll; liking and following it. */
function ThreadHead({ thread, member }: { thread: community.Thread; member: boolean }) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const subject = { type: common.SubjectType.COMMUNITY_THREAD, id: thread.id };
  const like = useLike(subject);
  const counts = useEngagement(subject).data;
  const liked = counts?.viewerReaction?.kind === engagement.ReactionKind.LIKE;
  const likes = counts ? counts.likeCount : Number(thread.likeCount);
  const following = thread.viewer?.following ?? false;

  const follow = async () => {
    try {
      const { following: now } = await boardClient().setThreadFollow({
        threadId: thread.id,
        following: !following,
      });
      patchThread(queryClient, thread.id, (each) =>
        each.viewer ? { ...each, viewer: { ...each.viewer, following: now } } : each,
      );
      void queryClient.invalidateQueries({ queryKey: communityKeys.followed });
    } catch (error) {
      complain(error);
    }
  };

  return (
    <View className="gap-3 border-b border-line bg-surface px-4 pt-4 pb-3">
      <Byline
        author={thread.author}
        pseudonym={thread.pseudonym}
        mine={thread.viewer?.mine}
        at={thread.createdAt}
      />
      <Text testID="thread-title" selectable className="text-xl font-black text-ink">
        {thread.title}
      </Text>
      {thread.state === community.PostState.HIDDEN && (
        <Text className="rounded-control bg-sun/30 px-3 py-2 text-xs font-bold text-ink">
          {thread.hiddenReason === community.HiddenReason.MUTED_WORD
            ? t("hidden.mutedWord")
            : t("hidden.reports")}
        </Text>
      )}
      <Body text={thread.body} mentions={thread.mentions} />
      <Images images={thread.images} />
      <PollCard thread={thread} canVote={member} />
      {thread.tags.length > 0 && (
        <Text className="text-sm font-bold text-brand">
          {thread.tags.map((tag) => `#${tag}`).join(" ")}
        </Text>
      )}
      <View className="flex-row items-center gap-4 pt-1">
        <Pressable
          testID="thread-like"
          accessibilityRole="button"
          accessibilityState={{ selected: liked }}
          onPress={() => void like(!liked)}
          className="flex-row items-center gap-1.5 py-1"
        >
          <Heart
            size={20}
            color={liked ? colors.pink : colors["ink-soft"]}
            fill={liked ? colors.pink : "transparent"}
          />
          <Text className="text-sm font-bold text-ink-soft">{likes}</Text>
        </Pressable>
        <Text className="text-sm font-bold text-ink-soft">
          {t("thread.replies", { count: thread.replyCount })}
        </Text>
        <View className="flex-1" />
        {member && (
          <Pressable
            testID="thread-follow"
            accessibilityRole="button"
            accessibilityState={{ selected: following }}
            onPress={() => void follow()}
            className="flex-row items-center gap-1.5 rounded-full bg-surface-2 px-3 py-1.5"
          >
            <Bell
              size={16}
              color={following ? colors.brand : colors.ink}
              fill={following ? colors.brand : "transparent"}
            />
            <Text className="text-sm font-bold text-ink">
              {following ? t("follow.following") : t("follow.action")}
            </Text>
          </Pressable>
        )}
      </View>
    </View>
  );
}

function ReplyRow({ reply, onReply }: { reply: community.Reply; onReply(): void }) {
  const { t, i18n } = useTranslation(NS);
  const quoted = reply.replyTo;
  return (
    <View testID={`reply-${reply.number}`} className="gap-2 border-b border-line px-4 py-3">
      <Byline
        author={reply.author}
        pseudonym={reply.pseudonym}
        mine={reply.viewer?.mine}
        at={reply.createdAt}
        extra={
          <>
            <Text className="text-xs font-bold text-ink-faint">{`#${reply.number}`}</Text>
            {reply.byThreadAuthor && (
              <Text className="text-xs font-black text-brand">{t("reply.byThreadAuthor")}</Text>
            )}
          </>
        }
      />
      {quoted && (
        <View className="rounded-control border-l-4 border-line bg-surface-2 px-3 py-1.5">
          <Text numberOfLines={2} className="text-xs text-ink-soft">
            {quoted.unavailable
              ? t("reply.quoteGone")
              : `${quoted.pseudonym ? pseudonymName(quoted.pseudonym, i18n.language) : (quoted.author?.displayName ?? "")}: ${quoted.excerpt}`}
          </Text>
        </View>
      )}
      <Body text={reply.body} mentions={reply.mentions} />
      <Images images={reply.images} />
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={t("reply.replyTo", { number: reply.number })}
        onPress={onReply}
        className="self-start py-1"
      >
        <Text className="text-xs font-bold text-ink-soft">{t("reply.action")}</Text>
      </Pressable>
    </View>
  );
}

/** Why a reply cannot be written here, from the server's `can_reply` and what the caller is. */
function cannotReplyKey(thread: community.Thread, member: boolean, timedOut: boolean): string {
  if (!member) return "reply.joinFirst";
  if (timedOut) return "reply.timedOut";
  if (thread.locked) return "reply.locked";
  if (thread.state === community.PostState.HIDDEN) return "reply.hiddenThread";
  return "reply.notAllowed";
}

function ReplyComposer({
  thread,
  member,
  timedOut,
  replyTo,
  onClearReplyTo,
}: {
  thread: community.Thread;
  member: boolean;
  timedOut: boolean;
  replyTo: community.Reply | null;
  onClearReplyTo(): void;
}) {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const [body, setBody] = useState("");
  const [sending, setSending] = useState(false);

  if (!thread.viewer?.canReply)
    return (
      <Text
        testID="reply-blocked"
        className="m-3 rounded-2xl bg-surface-2 px-4 py-3 text-center text-sm font-bold text-ink-soft"
      >
        {t(cannotReplyKey(thread, member, timedOut))}
      </Text>
    );

  const send = async () => {
    const text = body.trim();
    if (!text || sending) return;
    setSending(true);
    try {
      const { reply } = await boardClient().createReply({
        threadId: thread.id,
        body: text,
        replyToId: replyTo?.id ?? "",
      });
      setBody("");
      onClearReplyTo();
      if (reply) {
        upsertReply(queryClient, reply);
        if (reply.state === community.PostState.HIDDEN) Alert.alert(t("compose.held"));
      }
      void queryClient.invalidateQueries({ queryKey: communityKeys.thread(thread.shortId) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.thread(thread.id) });
    } catch (error) {
      complain(error);
    } finally {
      setSending(false);
    }
  };

  const replyName = replyTo
    ? replyTo.pseudonym
      ? pseudonymName(replyTo.pseudonym, i18n.language)
      : (replyTo.author?.displayName ?? "")
    : "";
  return (
    <View className="gap-1.5 border-t border-line bg-surface px-3 pt-2 pb-2">
      {thread.pseudonym && (
        <Text className="px-1 text-[11px] text-ink-soft">{`🎭 ${t("anonymous.replyHint")}`}</Text>
      )}
      {replyTo && (
        <View className="flex-row items-center gap-2 px-1">
          <Text numberOfLines={1} className="flex-1 text-xs font-bold text-brand">
            {t("compose.replyingTo", { number: replyTo.number, name: replyName })}
          </Text>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t("compose.cancelReplyTo")}
            onPress={onClearReplyTo}
          >
            <X size={16} color={colors["ink-soft"]} />
          </Pressable>
        </View>
      )}
      <View className="flex-row items-end gap-2">
        <TextInput
          testID="reply-input"
          accessibilityLabel={t("compose.reply")}
          multiline
          maxLength={2000}
          value={body}
          onChangeText={setBody}
          placeholder={t("compose.replyPlaceholder")}
          placeholderTextColor={colors["ink-faint"]}
          className="max-h-32 min-h-11 flex-1 rounded-2xl bg-surface-2 px-4 py-2.5 text-base text-ink"
        />
        <Pressable
          testID="reply-send"
          accessibilityRole="button"
          accessibilityLabel={t("compose.send")}
          disabled={!body.trim() || sending}
          onPress={() => void send()}
          className={`size-11 items-center justify-center rounded-full ${body.trim() ? "bg-brand" : "bg-surface-3"}`}
        >
          <SendHorizontal
            size={20}
            color={body.trim() ? colors["on-brand"] : colors["ink-faint"]}
          />
        </Pressable>
      </View>
    </View>
  );
}
