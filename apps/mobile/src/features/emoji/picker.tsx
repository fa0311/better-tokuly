import { type Catalog, type CatalogEmoji, findEmoji, searchEmoji } from "@tokuly/core/emoji";
import { Search, X } from "lucide-react-native";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  FlatList,
  Modal,
  Pressable,
  Text,
  TextInput,
  useWindowDimensions,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { EmptyState, Spinner } from "#/components/page";
import { useTheme } from "#/lib/theme";
import { useCatalog } from "./catalog";
import { NS, useDeck, useRecentEmoji } from "./data";
import { EmojiImage } from "./emoji-image";

const COLUMNS = 7;

type Line = { key: string; title: string } | { key: string; emojis: CatalogEmoji[] };

const rows = (key: string, emojis: CatalogEmoji[]): Line[] =>
  Array.from({ length: Math.ceil(emojis.length / COLUMNS) }, (_, index) => ({
    key: `${key}:${index}`,
    emojis: emojis.slice(index * COLUMNS, index * COLUMNS + COLUMNS),
  }));

/** The custom emoji of picker items (`:name:` each) that are in the catalogue, in their order. */
function customOf(catalog: Catalog, items: { emoji: string }[] | undefined): CatalogEmoji[] {
  return (items ?? []).flatMap((item) => {
    const found = item.emoji.startsWith(":") ? findEmoji(catalog, item.emoji) : undefined;
    return found ? [found] : [];
  });
}

/**
 * The custom emoji to choose from: a search by name and alias (Japanese
 * and English), or the deck, the ones used last and every category. Unicode
 * emoji are the keyboard's. `onPick` gets the tapped one.
 */
export function EmojiPanel({ onPick }: { onPick(entry: CatalogEmoji): void }) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const catalog = useCatalog();
  const deck = useDeck();
  const recent = useRecentEmoji();
  const [query, setQuery] = useState("");

  const lines = useMemo((): Line[] => {
    if (query.trim()) {
      const found = searchEmoji(query, catalog.emojis, []).flatMap((choice) =>
        choice.kind === "custom" ? [choice.custom] : [],
      );
      return found.length > 0 ? rows("found", found) : [];
    }
    const sections: Line[] = [];
    const add = (key: string, title: string, emojis: CatalogEmoji[]) => {
      if (emojis.length === 0) return;
      sections.push({ key: `${key}:title`, title }, ...rows(key, emojis));
    };
    add("deck", t("picker.deck"), customOf(catalog, deck.data?.items));
    add("recent", t("picker.recent"), customOf(catalog, recent.data));
    const byCategory = new Map<string, CatalogEmoji[]>();
    for (const entry of catalog.emojis)
      byCategory.set(entry.category, [...(byCategory.get(entry.category) ?? []), entry]);
    for (const category of catalog.categories)
      add(`c-${category.name}`, category.name, byCategory.get(category.name) ?? []);
    add("c-", t("picker.uncategorized"), byCategory.get("") ?? []);
    return sections;
  }, [query, catalog, deck.data, recent.data, t]);

  return (
    <View className="flex-1">
      <View className="mx-3 mb-2 h-10 flex-row items-center gap-2 rounded-full bg-surface-2 px-3">
        <Search size={16} color={colors["ink-faint"]} />
        <TextInput
          testID="emoji-search"
          value={query}
          onChangeText={setQuery}
          placeholder={t("picker.search")}
          placeholderTextColor={colors["ink-faint"]}
          autoCapitalize="none"
          autoCorrect={false}
          className="flex-1 text-base text-ink"
        />
        {query !== "" && (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t("picker.clear")}
            onPress={() => setQuery("")}
          >
            <X size={16} color={colors["ink-soft"]} />
          </Pressable>
        )}
      </View>
      {catalog.status !== "ready" && catalog.emojis.length === 0 ? (
        catalog.status === "error" ? (
          <EmptyState emoji="😵" title={t("browse.error")} body={t("browse.errorBody")} />
        ) : (
          <Spinner />
        )
      ) : lines.length === 0 ? (
        <EmptyState
          emoji="🔍"
          title={query ? t("picker.noResults") : t("browse.empty")}
          body={query ? undefined : t("browse.emptyBody")}
        />
      ) : (
        <EmojiLines lines={lines} onPick={onPick} />
      )}
    </View>
  );
}

function EmojiLines({ lines, onPick }: { lines: Line[]; onPick(entry: CatalogEmoji): void }) {
  const { width } = useWindowDimensions();
  const cell = Math.floor((width - 24) / COLUMNS);
  return (
    <FlatList
      data={lines}
      keyExtractor={(line) => line.key}
      keyboardShouldPersistTaps="handled"
      contentContainerClassName="px-3 pb-4"
      initialNumToRender={12}
      renderItem={({ item: line }) =>
        "title" in line ? (
          <Text className="px-1 pt-3 pb-1 text-xs font-black text-ink-soft">{line.title}</Text>
        ) : (
          <View className="flex-row">
            {line.emojis.map((entry) => (
              <Pressable
                key={entry.id}
                testID={`emoji-${entry.name}`}
                accessibilityRole="button"
                accessibilityLabel={`:${entry.name}:`}
                onPress={() => onPick(entry)}
                style={{ width: cell, height: cell }}
                className="items-center justify-center rounded-xl active:bg-surface-2"
              >
                <EmojiImage entry={entry} size={cell - 14} />
              </Pressable>
            ))}
          </View>
        )
      }
    />
  );
}

/**
 * Choosing a custom emoji to write, from the bottom of the screen; it stays
 * open so several can be added in a row. `onPick` gets `:name:`.
 */
export function EmojiPicker({
  visible,
  onPick,
  onClose,
}: {
  visible: boolean;
  onPick(code: string): void;
  onClose(): void;
}) {
  const { t } = useTranslation(NS);
  const { t: tc } = useTranslation("common");
  const insets = useSafeAreaInsets();
  const { height } = useWindowDimensions();
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      <View
        testID="emoji-picker"
        className="rounded-t-3xl bg-surface pt-3"
        style={{ height: Math.round(height * 0.55), paddingBottom: insets.bottom }}
      >
        <View className="flex-row items-center justify-between px-4 pb-2">
          <Text className="text-base font-black text-ink">{t("picker.title")}</Text>
          <Pressable
            testID="emoji-picker-done"
            accessibilityRole="button"
            onPress={onClose}
            className="rounded-full bg-surface-2 px-3 py-1.5"
          >
            <Text className="text-sm font-black text-ink">{tc("actions.close")}</Text>
          </Pressable>
        </View>
        {visible && <EmojiPanel onPick={(entry) => onPick(`:${entry.name}:`)} />}
      </View>
    </Modal>
  );
}
