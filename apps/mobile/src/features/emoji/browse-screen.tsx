import { useQueryClient } from "@tanstack/react-query";
import { findEmoji } from "@tokuly/core/emoji";
import { router } from "expo-router";
import { X } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, ScrollView, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { EmptyState, Row, Section, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { useCatalog } from "./catalog";
import { NS, saveDeck, useDeck } from "./data";
import { EmojiImage } from "./emoji-image";
import { EmojiPanel } from "./picker";

const openEmoji = (name: string) => router.push({ pathname: "/emoji/[name]", params: { name } });

/**
 * 絵文字: every custom emoji of the server to look through and search, and
 * the person's deck (what pickers and reactions offer first). Making and
 * sending in new ones is on the web screens.
 */
export function BrowseScreen() {
  const { t } = useTranslation(NS);
  const [tab, setTab] = useState<"all" | "deck">("all");
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="emoji-browse">
      <View className="px-4 pt-3 pb-3">
        <Tabs
          testID="emoji-tab"
          value={tab}
          onChange={setTab}
          items={[
            { value: "all", label: t("browse.sort.all") },
            { value: "deck", label: t("deck.title") },
          ]}
        />
      </View>
      {tab === "all" ? <EmojiPanel onPick={(entry) => openEmoji(entry.name)} /> : <Deck />}
    </SafeAreaView>
  );
}

function Deck() {
  const { t } = useTranslation(NS);
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const catalog = useCatalog();
  const deck = useDeck();
  const items = deck.data?.items ?? [];
  const remove = async (code: string) => {
    try {
      const kept = items.map((item) => item.emoji).filter((item) => item !== code);
      if (await saveDeck(queryClient, kept)) Alert.alert(t("deck.unpinned"));
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: NS }));
    }
  };
  return (
    <ScrollView contentContainerClassName="gap-4 px-4 pb-12">
      <Text className="text-sm text-ink-soft">{t("deck.lead")}</Text>
      {deck.isPending ? (
        <Spinner />
      ) : items.length === 0 ? (
        <EmptyState emoji="📌" title={t("deck.empty")} body={tm("emoji.deckEmptyBody")} />
      ) : (
        <View className="flex-row flex-wrap gap-2" testID="emoji-deck">
          {items.map((item) => {
            const custom = item.emoji.startsWith(":") ? findEmoji(catalog, item.emoji) : undefined;
            return (
              <View
                key={item.emoji}
                className="flex-row items-center gap-1 rounded-full bg-surface py-1 pr-1 pl-2"
              >
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={item.emoji}
                  disabled={!custom}
                  onPress={() => custom && openEmoji(custom.name)}
                >
                  {custom ? (
                    <EmojiImage entry={custom} size={28} />
                  ) : (
                    <Text className="text-2xl">{item.emoji}</Text>
                  )}
                </Pressable>
                <Pressable
                  testID={`deck-remove-${item.emoji}`}
                  accessibilityRole="button"
                  accessibilityLabel={t("deck.remove", { emoji: item.emoji })}
                  onPress={() => void remove(item.emoji)}
                  className="rounded-full p-1"
                >
                  <X size={14} color={colors["ink-soft"]} />
                </Pressable>
              </View>
            );
          })}
        </View>
      )}
      {deck.data?.isDefault && (
        <Text className="text-xs text-ink-faint">{t("deck.isDefault")}</Text>
      )}
      <Section>
        <Row
          testID="emoji-submit-web"
          label={tm("emoji.submit")}
          hint={tm("emoji.onWeb")}
          onPress={() => router.push(webScreenHref("/emoji/new-emoji"))}
        />
        <Row
          testID="emoji-submissions-web"
          label={tm("emoji.submissions")}
          hint={tm("emoji.onWeb")}
          onPress={() => router.push(webScreenHref("/emoji/submissions"))}
        />
      </Section>
    </ScrollView>
  );
}
