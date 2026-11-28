import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { chat, type common } from "@tokuly/api";
import { botHint, chatKeys } from "@tokuly/core/chat";
import { router } from "expo-router";
import { ChevronRight } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { SwitchRow } from "#/components/form-rows";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";

/**
 * Tokuly Bot, the AI account of the server: it answers in its direct
 * conversation, and in a group when mentioned (`@tokuly_bot`). The server
 * names it in the assistant settings (`bot`, unset where it does not work);
 * every bot account carries the bot badge (`UserSummary.bot`).
 */

const assistClient = () => api().service(chat.AssistService);
const settingsKey = ["chat", "assist-settings"] as const;

function useAssistSettings() {
  return useQuery({
    queryKey: settingsKey,
    queryFn: async () => (await assistClient().getAssistSettings({})).settings,
  });
}

/**
 * Shown only to the writer, in a group, while what they type mentions the bot
 * but it will not answer them (`botHint`): their switch or their AI assistant
 * is off. Never part of the conversation.
 */
export function BotMentionHint({ text }: { text: string }) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const settings = useAssistSettings();
  const enable = useMutation({
    mutationFn: () => assistClient().setBotEnabled({ enabled: true }),
    onSuccess: (response) => queryClient.setQueryData(settingsKey, response.settings),
    onError: (error) => Alert.alert(errorMessage(error, { namespace: "chat" })),
  });
  const hint = botHint(settings.data, text);
  if (!hint) return null;
  return (
    <View
      testID="bot-mention-hint"
      className="flex-row items-center gap-2 rounded-2xl bg-sky px-3 py-2"
    >
      <Text className="min-w-0 flex-1 text-xs font-bold text-[#17122b]">
        {hint === "botOff" ? t("bot.mentionOff") : t("bot.mentionAssistOff")}
      </Text>
      {hint === "botOff" && (
        <Pressable
          testID="bot-mention-turn-on"
          accessibilityRole="button"
          disabled={enable.isPending}
          onPress={() => enable.mutate()}
        >
          <Text className="text-xs font-black text-[#17122b] underline">{t("bot.turnOn")}</Text>
        </Pressable>
      )}
    </View>
  );
}

/** The badge next to the name of a bot account. */
export function BotBadge() {
  const { t } = useTranslation("chat");
  return (
    <View testID="bot-badge" className="rounded-full border-2 border-ink bg-sky px-1.5">
      <Text className="text-[10px] font-black text-[#17122b]">🤖 {t("bot.badge")}</Text>
    </View>
  );
}

/** The badge when `user` is a bot, else nothing. */
export function BotMark({ user }: { user?: common.UserSummary }) {
  return user?.bot && !user.deleted ? <BotBadge /> : null;
}

/**
 * "Talk to Tokuly Bot" at the top of a new chat: opens the conversation
 * with the bot, or offers the switch while it is off (the default under 18).
 * Nothing where the bot does not work.
 */
export function TalkToBotCard() {
  const { t } = useTranslation("chat");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const settings = useAssistSettings();
  const [opening, setOpening] = useState(false);
  const enable = useMutation({
    mutationFn: (enabled: boolean) => assistClient().setBotEnabled({ enabled }),
    onSuccess: (response) => queryClient.setQueryData(settingsKey, response.settings),
    onError: (error) => Alert.alert(errorMessage(error, { namespace: "chat" })),
  });
  const data = settings.data;
  if (!data?.bot) return null;

  const open = async () => {
    setOpening(true);
    try {
      const { conversation } = await assistClient().openBotConversation({});
      if (!conversation) return;
      queryClient.setQueryData(chatKeys.conversation(conversation.id), conversation, {
        updatedAt: 0,
      });
      router.replace({ pathname: "/chat/[id]", params: { id: conversation.id } });
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "chat" }));
    } finally {
      setOpening(false);
    }
  };

  return (
    <View className="mx-4 my-2 overflow-hidden rounded-2xl border-2 border-ink bg-surface">
      <Pressable
        testID="talk-to-bot"
        accessibilityRole="button"
        disabled={!data.botEnabled || opening}
        onPress={() => void open()}
        className={`flex-row items-center gap-3 px-4 py-3 ${data.botEnabled ? "" : "opacity-50"}`}
      >
        <Avatar user={data.bot} size="md" />
        <View className="min-w-0 flex-1">
          <View className="flex-row items-center gap-1.5">
            <Text numberOfLines={1} className="shrink font-black text-ink">
              {t("bot.talk")}
            </Text>
            <BotBadge />
          </View>
          <Text numberOfLines={1} className="text-sm font-bold text-ink-soft">
            {t("bot.talkLead")}
          </Text>
        </View>
        <ChevronRight size={20} color={colors["ink-faint"]} />
      </Pressable>
      {!data.botEnabled && (
        <SwitchRow
          testID="bot-enabled"
          label={t("bot.enabled")}
          hint={t("bot.enabledHint")}
          value={data.botEnabled}
          disabled={enable.isPending}
          onChange={(enabled) => enable.mutate(enabled)}
        />
      )}
    </View>
  );
}
