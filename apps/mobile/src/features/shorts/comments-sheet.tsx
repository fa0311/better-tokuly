import { FlashList } from "@shopify/flash-list";
import { type shorts, toMillis } from "@tokuly/api";
import { compactAge } from "@tokuly/core/format";
import { shortSubject } from "@tokuly/core/shorts";
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

const MAX_COMMENT = 1000;

/** The comments of a short, and a field to add one (while the short takes them). */
export function CommentsSheet({
  short,
  visible,
  onClose,
}: {
  short: shorts.Short;
  visible: boolean;
  onClose(): void;
}) {
  const { t } = useTranslation("shorts");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const subject = shortSubject(short);
  const comments = useComments(subject, visible);
  const write = useComment(subject);
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const rows = comments.data?.pages.flatMap((page) => page.comments) ?? [];
  const open = comments.data?.pages[0]?.commentsEnabled ?? short.commentsEnabled;
  const send = async () => {
    if (!text.trim()) return;
    setSending(true);
    if (await write(text.trim())) setText("");
    setSending(false);
  };
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      <KeyboardAvoidingView behavior="padding">
        <View
          testID="short-comments-sheet"
          className="h-[480px] rounded-t-3xl bg-surface pt-4"
          style={{ paddingBottom: insets.bottom + 8 }}
        >
          <Text className="px-5 pb-2 text-lg font-black text-ink">{t("comments.title")}</Text>
          {comments.isPending ? (
            <Spinner />
          ) : rows.length === 0 ? (
            <EmptyState emoji="💬" title={tm("shorts.noComments")} />
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
                    <Text className="text-base text-ink">{item.text}</Text>
                  </View>
                </View>
              )}
            />
          )}
          {open ? (
            <View className="flex-row items-center gap-2 border-line border-t px-4 pt-2">
              <TextInput
                testID="short-comment-input"
                value={text}
                maxLength={MAX_COMMENT}
                onChangeText={setText}
                placeholder={tm("shorts.commentPlaceholder")}
                placeholderTextColor={colors["ink-faint"]}
                className="flex-1 rounded-full bg-surface-2 px-4 py-2.5 text-base text-ink"
              />
              <Pressable
                testID="short-comment-send"
                accessibilityRole="button"
                accessibilityLabel={tm("shorts.commentSend")}
                disabled={!text.trim() || sending}
                onPress={() => void send()}
                className="p-2"
              >
                <Send color={colors.brand} size={24} />
              </Pressable>
            </View>
          ) : (
            <Text className="px-5 py-3 text-center text-sm text-ink-soft">
              {tm("shorts.commentsOff")}
            </Text>
          )}
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}
