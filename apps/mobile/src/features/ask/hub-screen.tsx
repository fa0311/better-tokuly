import { useQueryClient } from "@tanstack/react-query";
import type { ask } from "@tokuly/api";
import { askKeys } from "@tokuly/core/ask";
import { type Href, router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { SwitchRow } from "#/components/form-rows";
import { Page, Row, Section, Spinner } from "#/components/page";
import { useMe } from "#/features/session/store";
import { webScreenHref } from "#/features/web/handoff";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { boxAs, stillActive, useInboxCounts, useSettings, useUnseenFlames } from "./data";
import { showAskError } from "./parts";
import { ShareBoxButton } from "./share";

const open = (href: string) => router.push(href as Href);

/**
 * `/ask`: the viewer's own question box — open or not, its line, sharing it —
 * and the ways to the inbox, the Q&A page and the questions they sent.
 * Prompts, compliment polls and the box's settings open on the web.
 */
export function HubScreen() {
  const { t } = useTranslation("ask");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const handle = useMe()?.user?.handle ?? "";
  const settings = useSettings();
  const counts = useInboxCounts().data;
  const flames = useUnseenFlames();
  const isOpen = settings.data?.boxOpen ?? false;

  const setOpen = async (boxOpen: boolean) => {
    const { account, client } = boxAs();
    queryClient.setQueryData<ask.Settings>(askKeys.settings, (known) =>
      known ? { ...known, boxOpen } : known,
    );
    try {
      const saved = (await client.updateSettings({ boxOpen })).settings;
      if (stillActive(account)) queryClient.setQueryData(askKeys.settings, saved);
    } catch (error) {
      if (!stillActive(account)) return;
      showAskError(error);
      void queryClient.invalidateQueries({ queryKey: askKeys.settings });
    }
  };

  return (
    <Page testID="ask-hub">
      <View className="gap-3 rounded-card bg-surface p-4" style={sticker(colors)}>
        <View className="flex-row items-center gap-2">
          <Text className="flex-1 font-black text-ink">{t("hub.myBox")}</Text>
          {settings.data && (
            <View
              testID="ask-box-state"
              className={`rounded-full px-2.5 py-0.5 ${isOpen ? "bg-lime" : "bg-surface-3"}`}
            >
              <Text
                className={`text-xs font-black ${isOpen ? "" : "text-ink-soft"}`}
                style={isOpen ? { color: INK_ON_BRIGHT } : undefined}
              >
                {isOpen ? t("hub.open") : t("hub.closed")}
              </Text>
            </View>
          )}
        </View>
        {settings.isPending ? (
          <Spinner />
        ) : (
          <>
            <View
              className={`rounded-2xl px-4 py-3 ${isOpen ? "bg-sun" : "bg-surface-2"}`}
              style={sticker(colors, "sm")}
            >
              <Text
                className={`text-lg leading-6 font-black ${isOpen ? "" : "text-ink-soft"}`}
                style={isOpen ? { color: INK_ON_BRIGHT } : undefined}
              >
                💌 {settings.data?.boxPrompt || t("settings.promptPlaceholder")}
              </Text>
            </View>
            {!isOpen && (
              <Text className="px-1 text-sm font-bold text-ink-soft">{t("hub.closedLead")}</Text>
            )}
            <View className="-mx-4">
              <SwitchRow
                testID="ask-open"
                label={t("hub.openSwitch")}
                hint={t("hub.openHint")}
                value={isOpen}
                onChange={(next) => void setOpen(next)}
              />
            </View>
            <ShareBoxButton block />
          </>
        )}
      </View>

      <Section>
        <Row
          testID="ask-inbox-row"
          label={t("hub.inbox")}
          hint={counts?.unread ? t("hub.inboxNew", { count: counts.unread }) : t("hub.inboxLead")}
          badge={counts?.unread}
          onPress={() => open("/ask/inbox")}
        />
        <Row
          testID="ask-answers-row"
          label={t("hub.myAnswers")}
          hint={counts ? t("hub.myAnswersLead", { count: counts.answered }) : undefined}
          onPress={() => handle && open(`/ask/u/${handle}`)}
        />
        <Row
          testID="ask-sent-row"
          label={t("hub.sent")}
          hint={t("hub.sentLead")}
          onPress={() => open("/ask/sent")}
        />
      </Section>

      <Section lead={tm("ask.onWeb")}>
        <Row
          label={tm("ask.prompts")}
          hint={t("hub.promptsLead")}
          onPress={() => open(webScreenHref("/ask/prompts"))}
        />
        <Row
          label={tm("ask.polls")}
          hint={t("hub.pollsLead")}
          onPress={() => open(webScreenHref("/ask/polls"))}
        />
        <Row
          label={tm("ask.flames")}
          hint={t("hub.flamesLead")}
          badge={flames}
          onPress={() => open(webScreenHref("/ask/flames"))}
        />
        <Row label={tm("ask.settingsOnWeb")} onPress={() => open(webScreenHref("/ask/settings"))} />
      </Section>
    </Page>
  );
}
