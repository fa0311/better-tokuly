import { stamp } from "@tokuly/api";
import { router } from "expo-router";
import { LibraryBig, Search, X } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { FlatList, Pressable, ScrollView, Text, TextInput, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { EmptyState, Spinner } from "#/components/page";
import { useTheme } from "#/lib/theme";
import { NS, useShelf, useStoreHome } from "./data";
import { PackCard, PackRow } from "./parts";

/**
 * The stamp store: the team's picks, the ranking and what is new, the most
 * used tags, and a search over every published pack. A pack opens its page,
 * where a free one is added in a tap.
 */
export function StoreScreen() {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const [text, setText] = useState("");
  const [query, setQuery] = useState("");
  const [tag, setTag] = useState("");
  // The search follows typing a moment later.
  useEffect(() => {
    const timer = setTimeout(() => setQuery(text.trim()), 300);
    return () => clearTimeout(timer);
  }, [text]);

  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="stamp-store">
      <View className="flex-row items-center gap-2 px-4 pt-3 pb-2">
        <View className="h-11 flex-1 flex-row items-center gap-2 rounded-full bg-surface px-4">
          <Search size={18} color={colors["ink-faint"]} />
          <TextInput
            testID="stamp-search"
            value={text}
            onChangeText={setText}
            placeholder={t("search.placeholder")}
            placeholderTextColor={colors["ink-faint"]}
            returnKeyType="search"
            className="flex-1 text-base text-ink"
          />
          {text !== "" && (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t("search.clear")}
              onPress={() => setText("")}
            >
              <X size={18} color={colors["ink-soft"]} />
            </Pressable>
          )}
        </View>
        <Pressable
          testID="stamp-library"
          accessibilityRole="button"
          accessibilityLabel={t("library.title")}
          onPress={() => router.push("/stamps/library")}
          className="h-11 w-11 items-center justify-center rounded-full bg-surface"
        >
          <LibraryBig size={20} color={colors.ink} />
        </Pressable>
      </View>
      {query || tag ? (
        <Results query={query} tag={query ? "" : tag} onClearTag={() => setTag("")} />
      ) : (
        <Home onTag={setTag} />
      )}
    </SafeAreaView>
  );
}

function Home({ onTag }: { onTag(tag: string): void }) {
  const { t } = useTranslation(NS);
  const home = useStoreHome();
  if (home.isPending) return <Spinner />;
  if (!home.data) return <EmptyState emoji="😵" title={t("store.error")} />;
  const { featured, popular, newest, tags } = home.data;
  if (featured.length + popular.length + newest.length === 0)
    return <EmptyState emoji="🧸" title={t("store.empty")} body={t("store.emptyBody")} />;
  return (
    <ScrollView contentContainerClassName="gap-6 pt-2 pb-12">
      <Shelf title={t("store.featured")} packs={featured} />
      <Shelf title={t("store.popular")} packs={popular} />
      <Shelf title={t("store.new")} packs={newest} />
      {tags.length > 0 && (
        <View className="gap-2 px-4">
          <Text className="text-lg font-black text-ink">{t("browse.tags")}</Text>
          <View className="flex-row flex-wrap gap-2">
            {tags.map((item) => (
              <Pressable
                key={item.tag}
                accessibilityRole="button"
                onPress={() => onTag(item.tag)}
                className="rounded-full bg-surface px-3 py-1.5"
              >
                <Text className="font-bold text-brand-strong">#{item.tag}</Text>
              </Pressable>
            ))}
          </View>
        </View>
      )}
    </ScrollView>
  );
}

function Shelf({ title, packs }: { title: string; packs: stamp.PackSummary[] }) {
  if (packs.length === 0) return null;
  return (
    <View className="gap-2">
      <Text className="px-4 text-lg font-black text-ink">{title}</Text>
      <FlatList
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerClassName="gap-3 px-4"
        data={packs}
        keyExtractor={(pack) => pack.id}
        renderItem={({ item }) => <PackCard pack={item} />}
      />
    </View>
  );
}

function Results({ query, tag, onClearTag }: { query: string; tag: string; onClearTag(): void }) {
  const { t } = useTranslation(NS);
  const shelf = useShelf({
    section: stamp.StoreSection.POPULAR,
    tag,
    price: stamp.PriceFilter.UNSPECIFIED,
    query: query || undefined,
  });
  const packs = shelf.data?.pages.flatMap((page) => page.packs) ?? [];
  return (
    <FlatList
      testID="stamp-results"
      data={packs}
      keyExtractor={(pack) => pack.id}
      onEndReached={() => shelf.hasNextPage && !shelf.isFetchingNextPage && shelf.fetchNextPage()}
      ListHeaderComponent={
        tag ? (
          <Pressable
            accessibilityRole="button"
            onPress={onClearTag}
            className="mx-4 mb-2 self-start rounded-full bg-brand-soft px-3 py-1.5"
          >
            <Text className="font-black text-brand-strong">{t("browse.clearTag", { tag })} ✕</Text>
          </Pressable>
        ) : undefined
      }
      ListEmptyComponent={
        shelf.isPending ? (
          <Spinner />
        ) : (
          <EmptyState
            emoji="🔍"
            title={query ? t("browse.noResults", { query }) : t("browse.empty")}
            body={query ? t("browse.noResultsBody") : undefined}
          />
        )
      }
      ListFooterComponent={shelf.isFetchingNextPage ? <Spinner /> : undefined}
      renderItem={({ item }) => <PackRow pack={item} />}
    />
  );
}
