import { FlashList } from "@shopify/flash-list";
import { toMillis } from "@tokuly/api";
import type { Subject } from "@tokuly/core/engagement";
import { compactAge } from "@tokuly/core/format";
import { Send } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { KeyboardAvoidingView, Modal, Pressable, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Avatar } from "#/components/avatar";
import { EmptyState, Spinner } from "#/components/page";
import { useComment, useComments } from "#/features/engagement/data";
import i18n from "#/lib/i18n";
import { useTheme } from "#/lib/theme";
import { LinkedText } from "./description";

const MAX_COMMENT = 1000;

/**
 * The comments of a video, and a field to add one while the video takes
 * them. A time in a comment (`3:15`) jumps the player there.
 */
export function CommentsSheet({
  subject,
  enabled,
  visible,
  onClose,
  onSeek,
}: {
  subject: Subject;
  /** The video takes comments (`Video.comments_enabled`). */
  enabled: boolean;
  visible: boolean;
  onClose(): void;
  onSeek(seconds: number): void;
}) {
  const { t } = useTranslation("video");
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const comments = useComments(subject, visible);
  const write = useComment(subject);
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const rows = comments.data?.pages.flatMap((page) => page.comments) ?? [];
  const open = comments.data?.pages[0]?.commentsEnabled ?? enabled;
  const send = async () => {
    if (!text.trim()) return;
    setSending(true);
    if (await write(text.trim())) setText("");
    setSending(false);
  };
  const seek = (seconds: number) => {
    onClose();
    onSeek(seconds);
  };
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      <KeyboardAvoidingView behavior="padding">
        <View
          testID="video-comments-sheet"
          className="h-[520px] rounded-t-3xl bg-surface pt-4"
          style={{ paddingBottom: insets.bottom + 8 }}
        >
          <Text className="px-5 pb-2 text-lg font-black text-ink">{t("app.comments")}</Text>
          {comments.isPending ? (
            <Spinner />
          ) : rows.length === 0 ? (
            <EmptyState emoji="💬" title={t("app.noComments")} />
          ) : (
            <FlashList
              data={rows}
              keyExtractor={(row) => row.id}
              onEndReached={() => comments.hasNextPage && void comments.fetchNextPage()}
              renderItem={({ item }) => (
                <View className="flex-row gap-3 px-5 py-2.5">
                  <Avatar user={item.author?.user} size="sm" />
                  <View className="flex-1 gap-0.5">
                    <Text className="text-xs font-black text-ink-soft">
                      {item.author?.channel?.name ?? item.author?.user?.displayName} ·{" "}
                      {compactAge(i18n, toMillis(item.createdAt))}
                    </Text>
                    <LinkedText text={item.text} onSeek={seek} className="text-base text-ink" />
                  </View>
                </View>
              )}
            />
          )}
          {open ? (
            <View className="flex-row items-center gap-2 border-line border-t px-4 pt-2">
              <TextInput
                testID="video-comment-input"
                value={text}
                maxLength={MAX_COMMENT}
                onChangeText={setText}
                placeholder={t("app.commentPlaceholder")}
                placeholderTextColor={colors["ink-faint"]}
                className="flex-1 rounded-full bg-surface-2 px-4 py-2.5 text-base text-ink"
              />
              <Pressable
                testID="video-comment-send"
                accessibilityRole="button"
                accessibilityLabel={t("app.commentSend")}
                disabled={!text.trim() || sending}
                onPress={() => void send()}
                className="p-2"
              >
                <Send color={colors.brand} size={24} />
              </Pressable>
            </View>
          ) : (
            <Text className="px-5 py-3 text-center text-sm text-ink-soft">
              {t("app.commentsOff")}
            </Text>
          )}
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}
