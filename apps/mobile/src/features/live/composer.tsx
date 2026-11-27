import { useQueryClient } from "@tanstack/react-query";
import { errorReason, type live, toMillis } from "@tokuly/api";
import { CHAT_MAX_LENGTH, chatBlockReason, slowModeWait } from "@tokuly/core/live";
import { Gift, Hourglass, SendHorizontal } from "lucide-react-native";
import {
  type ComponentRef,
  type Ref,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, TextInput, View } from "react-native";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { chatClient } from "./data";
import { useNow } from "./parts";
import { liveChats } from "./room";

export interface ComposerHandle {
  /** Puts text (a mention) at the start of the field and focuses it. */
  insert(text: string): void;
}

/**
 * Where a viewer writes into the live chat, and the button for a gift. It
 * says why when writing is not possible (a timeout, a chat for subscribers
 * only) and counts slow mode down. The line goes out with the client of
 * the account that pressed send.
 */
export function ChatComposer({
  stream,
  onGift,
  ref,
}: {
  stream: live.Stream;
  /** Opens the gift sheet; left out where gifts cannot be sent. */
  onGift?(): void;
  ref?: Ref<ComposerHandle>;
}) {
  const { t, i18n } = useTranslation("live");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const input = useRef<ComponentRef<typeof TextInput>>(null);
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  /** A line on its way: a second press in the same frame (before `sending` shows) sends nothing. */
  const inFlight = useRef(false);
  /** When the last line went out, for slow mode. */
  const [sentAt, setSentAt] = useState<number | null>(null);
  const now = useNow();
  const streamId = stream.summary?.id ?? "";
  const viewer = stream.viewer;
  const exempt = Boolean(viewer?.isModerator);
  const slowSeconds = stream.chat?.slowModeSeconds ?? 0;
  const wait = exempt ? 0 : slowModeWait(slowSeconds, sentAt, now);
  const blocked = chatBlockReason(viewer, now);

  useImperativeHandle(ref, () => ({
    insert(mention) {
      setText((current) => (current.startsWith(mention) ? current : mention + current));
      input.current?.focus();
    },
  }));

  useEffect(() => {
    if (blocked) setText("");
  }, [blocked]);

  if (blocked) {
    const until = viewer?.chatBlockedUntil ? toMillis(viewer.chatBlockedUntil) : undefined;
    return (
      <Text
        testID="chat-blocked"
        className="mx-3 mb-3 rounded-2xl bg-surface-2 px-4 py-3 text-center text-sm font-bold text-ink-soft"
      >
        {blocked === "CHAT_TIMED_OUT" && until
          ? t("chat.blocked.CHAT_TIMED_OUT", {
              time: new Date(until).toLocaleTimeString(i18n.language, {
                hour: "2-digit",
                minute: "2-digit",
              }),
            })
          : t(`chat.blocked.${blocked}`, { defaultValue: t("chat.blocked.other") })}
      </Text>
    );
  }

  const trimmed = text.trim();
  const canSend = trimmed.length > 0 && wait === 0 && !sending;
  const send = async () => {
    if (!canSend || inFlight.current) return;
    inFlight.current = true;
    const client = chatClient();
    setSending(true);
    try {
      const { message } = await client.sendChatMessage({ streamId, text: trimmed, stampId: "" });
      setSentAt(Date.now());
      setText("");
      if (message) liveChats.addOwn(queryClient, message);
    } catch (error) {
      // The server's clock decides: wait the whole interval again.
      if (errorReason(error) === "SLOW_MODE") setSentAt(Date.now());
      Alert.alert(errorMessage(error, { namespace: "live" }));
    } finally {
      inFlight.current = false;
      setSending(false);
    }
  };

  return (
    <View className="border-t-2 border-line bg-surface px-3 pt-2 pb-2">
      {wait > 0 && (
        <View testID="slow-mode-wait" className="mb-1.5 flex-row items-center gap-1.5 px-1">
          <Hourglass size={13} color={colors["ink-soft"]} />
          <Text className="text-xs font-bold text-ink-soft">
            {t("chat.slowWait", { count: wait })}
          </Text>
        </View>
      )}
      <View className="flex-row items-center gap-1.5">
        <TextInput
          ref={input}
          testID="live-chat-input"
          value={text}
          onChangeText={setText}
          maxLength={CHAT_MAX_LENGTH}
          placeholder={
            slowSeconds && !exempt
              ? t("chat.placeholderSlow", { count: slowSeconds })
              : t("chat.placeholder")
          }
          placeholderTextColor={colors["ink-faint"]}
          accessibilityLabel={t("chat.inputLabel")}
          returnKeyType="send"
          submitBehavior="submit"
          onSubmitEditing={() => void send()}
          className="h-11 py-0 min-w-0 flex-1 rounded-full border-2 border-line bg-surface px-4 text-[15px] text-ink"
        />
        {onGift && (
          <Pressable
            testID="live-gift"
            accessibilityRole="button"
            accessibilityLabel={t("gift.open")}
            onPress={onGift}
            className="h-11 w-11 items-center justify-center rounded-full bg-sun"
          >
            <Gift size={20} color="#17122b" />
          </Pressable>
        )}
        <Pressable
          testID="live-chat-send"
          accessibilityRole="button"
          accessibilityLabel={t("chat.send")}
          accessibilityState={{ disabled: !canSend }}
          disabled={!canSend}
          onPress={() => void send()}
          className={`h-11 w-11 items-center justify-center rounded-full bg-brand ${canSend ? "" : "opacity-40"}`}
        >
          <SendHorizontal size={20} color={colors["on-brand"]} />
        </Pressable>
      </View>
    </View>
  );
}
