import { FlashList, type FlashListRef } from "@shopify/flash-list";
import { useQueryClient } from "@tanstack/react-query";
import { chat, type common, errorReason } from "@tokuly/api";
import { abilitiesOf, canWrite, isGroup, peerOf, type Row } from "@tokuly/core/chat";
import { router } from "expo-router";
import * as WebBrowser from "expo-web-browser";
import { ChevronLeft, Phone, Sparkles } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, AppState, KeyboardAvoidingView, Pressable, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { ActionSheet, type SheetAction } from "#/components/action-sheet";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { useMe } from "#/features/session/session";
import { webLink } from "#/lib/config";
import { errorMessage } from "#/lib/errors";
import { realtime } from "#/lib/realtime";
import { useTheme } from "#/lib/theme";
import { Composer } from "./composer";
import { ConversationAvatar } from "./conversation-row";
import {
  deleteMessage,
  markRead,
  respondToRequest,
  toggleReaction,
  useCapabilities,
  useConversation,
} from "./data";
import { useLive, useTyping } from "./live";
import { MessageBubble } from "./message-bubble";
import { useMessageWindow } from "./message-window";
import { discard, retry, useOutgoing } from "./outbox";
import { dayLabel, displayName, titleOf } from "./text";

/** "Has the conversation open" is repeated this often while it is on screen. */
const PRESENCE_EVERY_MS = 30_000;
/** Reactions one tap away; the web's picker has the rest. */
const QUICK_REACTIONS = ["❤️", "😂", "👍", "😮", "😢", "🎉"];
/** How close to the newest message still counts as "at the bottom". */
const AT_END_PX = 80;

/** Whether the app is in the foreground (reading needs the screen to be seen). */
function useActive(): boolean {
  const [active, setActive] = useState(AppState.currentState === "active");
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setActive(state === "active"),
    );
    return () => subscription.remove();
  }, []);
  return active;
}

/** One conversation: its messages, paged both ways, live, and the composer. */
export function ConversationScreen({ conversationId }: { conversationId: string }) {
  const { t } = useTranslation("chat");
  const me = useMe()?.user;
  const conversation = useConversation(conversationId);
  if (!me) return null;
  if (conversation.isError && !conversation.data)
    return (
      <SafeAreaView className="flex-1 bg-bg">
        <EmptyState emoji="🚪" title={t("gone.title")} body={t("gone.body")} />
        <View className="items-center">
          <Button variant="secondary" onPress={() => router.back()}>
            {t("gone.back")}
          </Button>
        </View>
      </SafeAreaView>
    );
  if (!conversation.data) return <Spinner />;
  return <Open conversation={conversation.data} me={me} />;
}

function Header({ conversation, myId }: { conversation: chat.Conversation; myId: string }) {
  const { t } = useTranslation("chat");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const typing = useTyping(conversation.id, myId);
  const names = typing
    .map((id) => displayName(conversation.members.find((member) => member.id === id)))
    .join(t("people.separator"));
  const onWeb = (title: string, body: string) =>
    Alert.alert(title, body, [
      { text: t("actions.cancel"), style: "cancel" },
      {
        text: tm("chat.openWeb"),
        onPress: () => void WebBrowser.openBrowserAsync(webLink(`/chat/${conversation.id}`)),
      },
    ]);
  const peer = peerOf(conversation, myId);
  return (
    <View className="flex-row items-center gap-2 border-b border-line bg-bg px-2 py-2">
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={t("actions.back")}
        onPress={() => router.back()}
        className="p-1"
      >
        <ChevronLeft size={26} color={colors.ink} />
      </Pressable>
      <Pressable
        className="min-w-0 flex-1 flex-row items-center gap-2"
        disabled={!peer}
        onPress={() =>
          peer && router.push({ pathname: "/u/[handle]", params: { handle: peer.handle } })
        }
      >
        <ConversationAvatar conversation={conversation} myId={myId} size="sm" />
        <View className="min-w-0 flex-1">
          <Text testID="conversation-title" numberOfLines={1} className="font-black text-ink">
            {titleOf(conversation, myId)}
          </Text>
          {typing.length > 0 ? (
            <Text
              testID="conversation-typing"
              numberOfLines={1}
              className="text-xs font-bold text-brand"
            >
              {isGroup(conversation) ? t("typing.named", { names }) : t("typing.short")}
            </Text>
          ) : isGroup(conversation) ? (
            <Text className="text-xs font-bold text-ink-soft">
              {t("group.memberCount", { count: conversation.memberCount })}
            </Text>
          ) : null}
        </View>
      </Pressable>
      <Pressable
        testID="conversation-assist"
        accessibilityRole="button"
        accessibilityLabel={tm("chat.assist.title")}
        onPress={() => onWeb(tm("chat.assist.title"), tm("chat.assist.body"))}
        className="p-2"
      >
        <Sparkles size={22} color={colors.ink} />
      </Pressable>
      <Pressable
        testID="conversation-call"
        accessibilityRole="button"
        accessibilityLabel={tm("chat.calls.title")}
        onPress={() => onWeb(tm("chat.calls.title"), tm("chat.calls.body"))}
        className="p-2"
      >
        <Phone size={22} color={colors.ink} />
      </Pressable>
    </View>
  );
}

function Open({ conversation, me }: { conversation: chat.Conversation; me: common.UserSummary }) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const active = useActive();
  const { editWindowSeconds } = useCapabilities();
  const window = useMessageWindow(conversation.id, conversation, me);
  const outgoing = useOutgoing(conversation.id);
  const cursors = useLive((state) => state.cursors[conversation.id]);
  const list = useRef<FlashListRef<Row>>(null);
  const [atEnd, setAtEnd] = useState(true);
  const [replyTo, setReplyTo] = useState<chat.Message | null>(null);
  const [editing, setEditing] = useState<chat.Message | null>(null);
  const [menuFor, setMenuFor] = useState<chat.Message | null>(null);

  const id = conversation.id;
  const viewer = conversation.viewer;
  const group = isGroup(conversation);
  const accepted = viewer?.requestState === chat.RequestState.ACCEPTED;
  const pendingRequest = viewer?.requestState === chat.RequestState.PENDING;
  const writable = canWrite(conversation);

  // Co-presence: "has the conversation open", repeated while it is on screen.
  useEffect(() => {
    if (!active) return;
    const client = realtime();
    const announce = () => client.sendConversationPresence(id, true);
    announce();
    const timer = setInterval(announce, PRESENCE_EVERY_MS);
    const off = client.onFrame((frame) => frame.body.case === "welcome" && announce());
    return () => {
      clearInterval(timer);
      off();
      client.sendConversationPresence(id, false);
    };
  }, [id, active]);

  // Reading: at the bottom of a conversation on screen, what arrives is read as it comes.
  const reading = active && atEnd && !window.hasNewer && !window.loading;
  useEffect(() => {
    useLive.getState().setReading(reading ? id : null);
    return () => useLive.getState().setReading(null);
  }, [reading, id]);
  const newest = window.newest;
  const readSeq = viewer?.readSeq;
  useEffect(() => {
    if (!reading || !newest || readSeq === undefined || newest.seq <= readSeq) return;
    // A burst of arrivals is one request.
    const timer = setTimeout(() => void markRead(queryClient, id, newest.id).catch(() => {}), 150);
    return () => clearTimeout(timer);
  }, [reading, newest, readSeq, id, queryClient]);

  const readLabel = (message: chat.Message): string | undefined => {
    if (group)
      return message.readCount > 0
        ? t("message.readCount", { count: message.readCount })
        : undefined;
    const peer = peerOf(conversation, me.id);
    const peerSeq = peer ? cursors?.[peer.id] : undefined;
    return (peerSeq !== undefined && peerSeq >= message.seq) || message.readCount > 0
      ? t("message.read")
      : undefined;
  };

  const run = (work: Promise<unknown>) =>
    work.catch((error) =>
      Alert.alert(
        errorReason(error) === "MESSAGE_NOT_FOUND"
          ? t("message.notAvailable")
          : errorMessage(error, { namespace: "chat" }),
      ),
    );

  const abilities = menuFor
    ? abilitiesOf(menuFor, {
        myId: me.id,
        role: viewer?.role,
        writable,
        accepted,
        editWindowSeconds,
      })
    : null;
  const actions: SheetAction[] = [];
  if (menuFor && abilities) {
    const message = menuFor;
    if (abilities.reply)
      actions.push({ key: "reply", label: t("message.reply"), onPress: () => setReplyTo(message) });
    if (abilities.edit)
      actions.push({ key: "edit", label: t("message.edit"), onPress: () => setEditing(message) });
    if (abilities.delete)
      actions.push({
        key: "delete",
        label: t("message.delete"),
        destructive: true,
        onPress: () =>
          Alert.alert(
            t("message.deleteTitle"),
            message.sender?.id === me.id ? t("message.deleteLead") : t("message.deleteOthersLead"),
            [
              { text: t("actions.cancel"), style: "cancel" },
              {
                text: t("message.delete"),
                style: "destructive",
                onPress: () => void run(deleteMessage(queryClient, message.id)),
              },
            ],
          ),
      });
  }

  const renderRow = ({ item }: { item: Row }) => {
    switch (item.type) {
      case "day":
        return (
          <Text className="py-3 text-center text-xs font-black text-ink-faint">
            {dayLabel(item.at)}
          </Text>
        );
      case "unread":
        return (
          <View className="flex-row items-center gap-2 px-6 py-2">
            <View className="h-px flex-1 bg-brand" />
            <Text className="text-xs font-black text-brand">{t("message.unreadFrom")}</Text>
            <View className="h-px flex-1 bg-brand" />
          </View>
        );
      case "message": {
        const item_ = outgoing.find((entry) => entry.clientId === item.message.clientId);
        return (
          <MessageBubble
            message={item.message}
            myId={me.id}
            first={item.first}
            last={item.last}
            group={group}
            readLabel={item.message.sender?.id === me.id ? readLabel(item.message) : undefined}
            pending={item.pending}
            onLongPress={() => setMenuFor(item.message)}
            onReact={(emoji) => void run(toggleReaction(queryClient, item.message, emoji))}
            onRetry={() => {
              if (!item_) return;
              Alert.alert(item_.error ?? t("message.failed"), undefined, [
                { text: t("message.discard"), style: "destructive", onPress: () => discard(item_) },
                { text: t("message.retry"), onPress: () => retry(queryClient, item_) },
              ]);
            }}
          />
        );
      }
    }
  };

  return (
    <SafeAreaView edges={["top", "bottom"]} className="flex-1 bg-bg" testID="conversation-screen">
      <KeyboardAvoidingView
        className="flex-1"
        // Android draws edge to edge: the window is not resized for the keyboard, so both pad.
        behavior="padding"
      >
        <Header conversation={conversation} myId={me.id} />
        {window.loading ? (
          <Spinner />
        ) : (
          <FlashList
            ref={list}
            testID="message-list"
            data={window.rows}
            keyExtractor={(row) => row.key}
            getItemType={(row) => row.type}
            renderItem={renderRow}
            maintainVisibleContentPosition={{
              startRenderingFromBottom: true,
              autoscrollToBottomThreshold: 0.2,
            }}
            onStartReached={window.loadOlder}
            onStartReachedThreshold={0.5}
            onEndReached={window.loadNewer}
            onScroll={(event) => {
              const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
              setAtEnd(
                contentOffset.y + layoutMeasurement.height >= contentSize.height - AT_END_PX,
              );
            }}
            scrollEventThrottle={100}
            ListHeaderComponent={window.hasOlder ? <Spinner /> : <View className="h-3" />}
            ListFooterComponent={<View className="h-3" />}
            ListEmptyComponent={
              <EmptyState
                emoji={group ? "🎉" : "👋"}
                title={
                  group
                    ? t("empty.group.title")
                    : t("empty.direct.title", { name: titleOf(conversation, me.id) })
                }
                body={group ? t("empty.group.body") : t("empty.direct.body")}
              />
            }
          />
        )}
        {pendingRequest ? (
          <View className="gap-2 border-t border-line bg-surface px-4 py-3">
            <Text className="font-black text-ink">
              {t("request.title", { name: titleOf(conversation, me.id) })}
            </Text>
            <Text className="text-sm text-ink-soft">{t("request.lead")}</Text>
            <View className="flex-row gap-2">
              <Button
                size="sm"
                testID="request-accept"
                onPress={() => void run(respondToRequest(queryClient, id, true))}
              >
                {t("request.accept")}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                onPress={() =>
                  void run(respondToRequest(queryClient, id, false).then(() => router.back()))
                }
              >
                {t("request.decline")}
              </Button>
            </View>
          </View>
        ) : writable ? (
          <Composer
            conversationId={id}
            myId={me.id}
            replyTo={replyTo}
            editing={editing}
            textOnly={(viewer?.requestMessagesLeft ?? -1) >= 0}
            onDone={() => {
              setReplyTo(null);
              setEditing(null);
              list.current?.scrollToEnd({ animated: true });
            }}
          />
        ) : (
          <Text className="border-t border-line bg-surface px-4 py-4 text-center text-sm font-bold text-ink-soft">
            {t("composer.readOnly")}
          </Text>
        )}
      </KeyboardAvoidingView>
      <ActionSheet visible={menuFor !== null} actions={actions} onClose={() => setMenuFor(null)}>
        {abilities?.react && menuFor && (
          <View className="flex-row justify-around px-2 pb-2">
            {QUICK_REACTIONS.map((emoji) => (
              <Pressable
                key={emoji}
                testID={`react-${emoji}`}
                accessibilityRole="button"
                accessibilityLabel={t("message.reactWith", { emoji })}
                onPress={() => {
                  const message = menuFor;
                  setMenuFor(null);
                  void run(toggleReaction(queryClient, message, emoji));
                }}
                className="h-12 w-12 items-center justify-center rounded-full bg-surface-2"
              >
                <Text className="text-2xl">{emoji}</Text>
              </Pressable>
            ))}
          </View>
        )}
      </ActionSheet>
    </SafeAreaView>
  );
}
