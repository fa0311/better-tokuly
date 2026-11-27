import { useQueryClient } from "@tanstack/react-query";
import { toDate } from "@tokuly/api";
import { DECK_MAX, shortcode, toCatalogEmoji } from "@tokuly/core/emoji";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Page, Spinner } from "#/components/page";
import { errorMessage } from "#/lib/errors";
import { NS, saveDeck, useDeck, useEmojiDetail } from "./data";
import { EmojiImage } from "./emoji-image";

/**
 * One custom emoji: its picture (a sensitive one blurred until tapped), its
 * code, aliases, category, who made it and its licence, and pinning it to
 * the deck or taking it off.
 */
export function DetailScreen({ name }: { name: string }) {
  const { t, i18n } = useTranslation(NS);
  const detail = useEmojiDetail(name);
  const [reveal, setReveal] = useState(false);
  if (detail.isPending) return <Spinner />;
  const data = detail.data;
  const emoji = data?.emoji;
  if (!data || !emoji)
    return (
      <EmptyState
        emoji="🫥"
        title={t("detail.notFound")}
        body={t("detail.notFoundBody", { code: shortcode(name) })}
        action={
          <Button size="sm" onPress={() => router.replace("/emoji")}>
            {t("detail.toCatalog")}
          </Button>
        }
      />
    );
  const entry = toCatalogEmoji(emoji);
  const code = shortcode(entry.name);
  const facts: [string, string][] = [
    [t("fields.aliasesShort"), entry.aliases.join("、") || t("detail.none")],
    [t("fields.category"), entry.category || t("browse.uncategorized")],
    [t("detail.author"), data.author?.displayName || data.author?.handle || t("detail.noAuthor")],
    [t("fields.licenseShort"), data.license || t("detail.none")],
  ];
  if (emoji.addedAt)
    facts.push([t("detail.added"), toDate(emoji.addedAt)?.toLocaleDateString(i18n.language) ?? ""]);
  return (
    <Page testID="emoji-detail">
      <View className="items-center gap-3 rounded-card bg-surface p-6">
        <Pressable
          accessibilityRole="button"
          disabled={!entry.sensitive}
          onPress={() => setReveal(!reveal)}
        >
          <EmojiImage entry={entry} size={128} reveal={reveal} />
        </Pressable>
        {entry.sensitive && (
          <Text className="text-xs text-ink-soft">
            {reveal ? t("detail.hideSensitive") : t("detail.showSensitive")}
          </Text>
        )}
        <Text selectable className="text-xl font-black text-ink">
          {code}
        </Text>
        <View className="flex-row flex-wrap justify-center gap-1.5">
          {entry.animated && <Flag label={t("flags.animated")} />}
          {entry.sensitive && <Flag label={t("flags.sensitive")} />}
          {entry.localOnly && <Flag label={t("flags.localOnly")} />}
        </View>
        <PinButton code={code} />
      </View>
      <View className="overflow-hidden rounded-card bg-surface">
        {facts.map(([label, value]) => (
          <View key={label} className="flex-row gap-3 border-b border-line px-4 py-3">
            <Text className="w-28 text-sm font-bold text-ink-soft">{label}</Text>
            <Text selectable className="flex-1 text-sm text-ink">
              {value}
            </Text>
          </View>
        ))}
      </View>
    </Page>
  );
}

function Flag({ label }: { label: string }) {
  return (
    <Text className="rounded-full bg-surface-2 px-2.5 py-1 text-xs font-black text-ink-soft">
      {label}
    </Text>
  );
}

/** Pins the emoji to the front of the deck, or takes it off. */
function PinButton({ code }: { code: string }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const deck = useDeck();
  const [busy, setBusy] = useState(false);
  if (!deck.data) return null;
  const codes = deck.data.items.map((item) => item.emoji);
  const pinned = codes.includes(code);
  const change = async () => {
    if (!pinned && codes.length >= DECK_MAX) return Alert.alert(t("errors.DECK_TOO_LONG"));
    setBusy(true);
    try {
      const next = pinned ? codes.filter((item) => item !== code) : [code, ...codes];
      if (await saveDeck(queryClient, next))
        Alert.alert(pinned ? t("deck.unpinned") : t("deck.pinned"));
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: NS }));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Button
      testID={pinned ? "emoji-unpin" : "emoji-pin"}
      variant={pinned ? "ghost" : "primary"}
      loading={busy}
      onPress={() => void change()}
    >
      {pinned ? t("deck.unpin") : t("deck.pin")}
    </Button>
  );
}
