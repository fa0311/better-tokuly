import { type chat, toMillis } from "@tokuly/api";
import { isGroup, isMuted, peerOf } from "@tokuly/core/chat";
import { router } from "expo-router";
import { BellOff, Pin } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { useTheme } from "#/lib/theme";
import { listTime, previewText, titleOf } from "./text";

/** The face of a conversation: the other person, or the group's icon (its name's initial without one). */
export function ConversationAvatar({
  conversation,
  myId,
  size = "md",
}: {
  conversation: chat.Conversation;
  myId: string;
  size?: "sm" | "md";
}) {
  const user = isGroup(conversation)
    ? {
        id: conversation.id,
        displayName: titleOf(conversation, myId),
        handle: "",
        avatar: conversation.icon,
        deleted: false,
      }
    : peerOf(conversation, myId);
  return <Avatar user={user} size={size} />;
}

/** One conversation in a list: who, the newest message, when, and how much is unread. */
export function ConversationRow({
  conversation,
  myId,
  onLongPress,
}: {
  conversation: chat.Conversation;
  myId: string;
  onLongPress(): void;
}) {
  const { t } = useTranslation("chat");
  const { colors } = useTheme();
  const viewer = conversation.viewer;
  const unread = viewer?.unreadCount ?? 0;
  const muted = isMuted(viewer);
  const title = titleOf(conversation, myId);
  return (
    <Pressable
      testID={`conversation-${title}`}
      accessibilityRole="button"
      accessibilityHint={unread > 0 ? String(unread) : undefined}
      onPress={() => router.push({ pathname: "/chat/[id]", params: { id: conversation.id } })}
      onLongPress={onLongPress}
      className="flex-row items-center gap-3 border-b border-line bg-surface px-4 py-3"
    >
      <ConversationAvatar conversation={conversation} myId={myId} />
      <View className="min-w-0 flex-1 gap-0.5">
        <View className="flex-row items-center gap-1.5">
          <Text numberOfLines={1} className="shrink font-black text-ink">
            {title}
          </Text>
          {viewer?.pinned && (
            <Pin size={13} color={colors["ink-faint"]} accessibilityLabel={t("list.pinned")} />
          )}
          {muted && (
            <BellOff size={13} color={colors["ink-faint"]} accessibilityLabel={t("list.muted")} />
          )}
          <Text className="ml-auto text-xs font-bold text-ink-faint">
            {listTime(toMillis(conversation.lastActivityAt))}
          </Text>
        </View>
        <View className="flex-row items-center gap-2">
          <Text
            numberOfLines={1}
            className={`flex-1 text-sm ${unread > 0 ? "font-black text-ink" : "font-bold text-ink-soft"}`}
          >
            {conversation.activeCallId
              ? t("list.inCall")
              : previewText(conversation.lastMessage, myId, isGroup(conversation))}
          </Text>
          {unread > 0 && (
            <View
              testID={`unread-${title}`}
              className={`min-w-6 items-center rounded-full px-2 py-0.5 ${muted ? "bg-ink-faint" : "bg-brand"}`}
            >
              <Text className="text-xs font-black text-on-brand">
                {(viewer?.unreadMentionCount ?? 0) > 0 ? "@" : ""}
                {unread > 99 ? "99+" : unread}
              </Text>
            </View>
          )}
        </View>
      </View>
    </Pressable>
  );
}
