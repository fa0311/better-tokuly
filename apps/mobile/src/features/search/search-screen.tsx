import { FlashList } from "@shopify/flash-list";
import {
  idleSuggestions,
  KINDS,
  type Kind,
  kindSlug,
  MAX_QUERY_LENGTH,
  type SearchSuggestion,
  typedSuggestions,
} from "@tokuly/core/search";
import { Clock, Search, TrendingUp, X } from "lucide-react-native";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, ScrollView, Text, TextInput, View } from "react-native";
import { EmptyState, ErrorNote, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import {
  type Asked,
  useHistoryActions,
  useRecentSearches,
  useResults,
  useSuggest,
  useTrending,
} from "./data";
import { HitRow } from "./hit-row";
import { openHit } from "./open";

/** What a search started with: a link's query and tab (not remembered), or nothing. */
export interface SearchStart {
  query?: string;
  kind?: Kind;
}

/**
 * Search: recent searches and trends before typing, typeahead while typing,
 * and the results by kind once a query is asked for. What the server returns
 * is all there is: blocked people and content the viewer may not see never
 * reach the phone.
 */
export function SearchScreen({ start }: { start: SearchStart }) {
  const { t } = useTranslation("search");
  const { colors } = useTheme();
  const [text, setText] = useState(start.query ?? "");
  const [asked, setAsked] = useState<Asked & { kind?: Kind }>({
    query: start.query ?? "",
    kind: start.kind,
    remember: false,
    at: 0,
  });
  const submit = (query: string) => {
    const trimmed = query.trim();
    if (!trimmed) return;
    setText(trimmed);
    setAsked({ query: trimmed, kind: undefined, remember: true, at: Date.now() });
  };
  const typing = text.trim() !== asked.query;
  return (
    <View className="flex-1 bg-bg" testID="search-screen">
      <View className="mx-4 mt-3 mb-2 h-11 flex-row items-center gap-2 rounded-full bg-surface px-4">
        <Search size={18} color={colors["ink-faint"]} />
        <TextInput
          testID="search-input"
          accessibilityLabel={t("box.label")}
          placeholder={t("box.placeholder")}
          placeholderTextColor={colors["ink-faint"]}
          value={text}
          onChangeText={setText}
          onSubmitEditing={() => submit(text)}
          maxLength={MAX_QUERY_LENGTH}
          returnKeyType="search"
          autoCapitalize="none"
          autoCorrect={false}
          autoFocus={!start.query}
          className="h-11 flex-1 text-base text-ink"
        />
        {text ? (
          <Pressable
            testID="search-clear"
            accessibilityRole="button"
            accessibilityLabel={t("box.clear")}
            hitSlop={8}
            onPress={() => setText("")}
          >
            <X size={18} color={colors["ink-soft"]} />
          </Pressable>
        ) : null}
      </View>
      {!text.trim() ? (
        <Idle onSearch={submit} />
      ) : typing ? (
        <Typeahead text={text} onSearch={submit} />
      ) : (
        <Results
          asked={asked}
          kind={asked.kind}
          onKind={(kind) => setAsked({ ...asked, kind })}
          onSearch={submit}
        />
      )}
    </View>
  );
}

/** A tappable line of a suggestion list. */
function Line({
  icon,
  label,
  detail,
  onPress,
  onForget,
  testID,
}: {
  icon: ReactNode;
  label: string;
  detail?: string;
  onPress(): void;
  onForget?(): void;
  testID: string;
}) {
  const { t } = useTranslation("search");
  const { colors } = useTheme();
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      onPress={onPress}
      className="min-h-12 flex-row items-center gap-3 border-b border-line bg-surface px-4 py-2 active:bg-surface-2"
    >
      {icon}
      <Text numberOfLines={1} className="flex-1 text-base font-bold text-ink">
        {label}
      </Text>
      {detail ? <Text className="text-xs font-bold text-ink-soft">{detail}</Text> : null}
      {onForget ? (
        <Pressable
          testID={`${testID}-forget`}
          accessibilityRole="button"
          accessibilityLabel={t("recent.forget", { query: label })}
          hitSlop={8}
          onPress={onForget}
        >
          <X size={16} color={colors["ink-faint"]} />
        </Pressable>
      ) : null}
    </Pressable>
  );
}

function Heading({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <View className="flex-row items-center justify-between gap-2 px-4 pt-4 pb-1">
      <Text className="text-xs font-black text-ink-soft">{title}</Text>
      {children}
    </View>
  );
}

/** One of the suggestions in a list. */
function SuggestionLine({
  item,
  onSearch,
  onForget,
}: {
  item: SearchSuggestion;
  onSearch(query: string): void;
  onForget(id: string): void;
}) {
  const { t } = useTranslation("search");
  const { colors } = useTheme();
  if (item.type === "hit")
    return <HitRow hit={item.hit} onPress={() => openHit(item.hit, onSearch)} />;
  if (item.type === "search")
    return (
      <Line
        testID="search-submit"
        icon={<Search size={18} color={colors.brand} />}
        label={t("suggest.searchFor", { query: item.query })}
        onPress={() => onSearch(item.query)}
      />
    );
  const recentId = item.recentId;
  return (
    <Line
      testID={`search-${item.id}`}
      icon={
        item.source === "trending" ? (
          <TrendingUp size={18} color={colors.pink} />
        ) : item.source === "recent" ? (
          <Clock size={18} color={colors["ink-faint"]} />
        ) : (
          <Search size={18} color={colors["ink-faint"]} />
        )
      }
      label={item.query}
      detail={item.rank ? `${item.rank}` : undefined}
      onPress={() => onSearch(item.query)}
      onForget={recentId ? () => onForget(recentId) : undefined}
    />
  );
}

/** Before typing: the person's own recent searches (with what they can do with them), then trends. */
function Idle({ onSearch }: { onSearch(query: string): void }) {
  const { t } = useTranslation("search");
  const recent = useRecentSearches();
  const trending = useTrending();
  const history = useHistoryActions();
  const searches = recent.data?.searches ?? [];
  const paused = recent.data?.historyPaused ?? false;
  const list = idleSuggestions(searches, trending.data?.queries ?? [], {
    recent: 10,
    trending: 10,
  });
  const recents = list.filter((item) => item.group === "recent");
  const trends = list.filter((item) => item.group === "trending");
  const confirmClear = () => {
    // The account is the one that tapped, even if another is signed in by the time it is confirmed.
    const clear = history.clear();
    Alert.alert(t("recent.clearTitle"), t("recent.clearBody"), [
      { text: t("common:actions.cancel"), style: "cancel" },
      { text: t("recent.clearConfirm"), style: "destructive", onPress: clear },
    ]);
  };
  return (
    <ScrollView keyboardShouldPersistTaps="handled" contentContainerClassName="pb-12">
      <Heading title={paused ? `${t("recent.title")} · ${t("recent.paused")}` : t("recent.title")}>
        <View className="flex-row gap-3">
          <Pressable
            testID="search-history-pause"
            accessibilityRole="button"
            hitSlop={6}
            onPress={() => history.setPaused(!paused)}
          >
            <Text className="text-xs font-black text-brand-strong">
              {paused ? t("recent.resume") : t("recent.pause")}
            </Text>
          </Pressable>
          {searches.length > 0 && (
            <Pressable
              testID="search-history-clear"
              accessibilityRole="button"
              hitSlop={6}
              onPress={confirmClear}
            >
              <Text className="text-xs font-black text-danger">{t("recent.clear")}</Text>
            </Pressable>
          )}
        </View>
      </Heading>
      {recent.isPending ? (
        <Spinner />
      ) : recents.length ? (
        recents.map((item) => (
          <SuggestionLine key={item.id} item={item} onSearch={onSearch} onForget={history.forget} />
        ))
      ) : (
        <Text className="px-4 py-3 text-sm text-ink-soft">
          {paused ? t("recent.emptyPaused") : t("recent.empty")}
        </Text>
      )}
      <Heading title={t("trending.title")} />
      {trending.isPending ? (
        <Spinner />
      ) : trends.length ? (
        trends.map((item) => (
          <SuggestionLine key={item.id} item={item} onSearch={onSearch} onForget={history.forget} />
        ))
      ) : (
        <Text className="px-4 py-3 text-sm text-ink-soft">{t("trending.empty")}</Text>
      )}
    </ScrollView>
  );
}

/** While typing: completions, people and things to jump to, and "search for this". */
function Typeahead({ text, onSearch }: { text: string; onSearch(query: string): void }) {
  const suggest = useSuggest(text);
  const history = useHistoryActions();
  return (
    <ScrollView
      testID="search-suggestions"
      keyboardShouldPersistTaps="handled"
      contentContainerClassName="pb-12"
    >
      {typedSuggestions(text, suggest.data).map((item) => (
        <SuggestionLine key={item.id} item={item} onSearch={onSearch} onForget={history.forget} />
      ))}
    </ScrollView>
  );
}

type Tab = "all" | string;

/** The results of an asked query, one tab per kind that has any. */
function Results({
  asked,
  kind,
  onKind,
  onSearch,
}: {
  asked: Asked;
  kind: Kind | undefined;
  onKind(kind: Kind | undefined): void;
  onSearch(query: string): void;
}) {
  const { t } = useTranslation("search");
  // The counts come with the first page of every kind; a tab keeps the ones it was chosen from.
  const all = useResults(asked, undefined);
  const inTab = useResults(asked, kind, kind !== undefined);
  const results = kind === undefined ? all : inTab;
  const counts = all.data?.pages[0]?.counts ?? [];
  const found = KINDS.filter((each) =>
    counts.some((count) => count.kind === each && count.count > 0),
  );
  const tabs: Kind[] = kind !== undefined && !found.includes(kind) ? [kind, ...found] : found;
  const hits = results.data?.pages.flatMap((page) => page.hits) ?? [];
  return (
    <View className="flex-1">
      {tabs.length > 0 && (
        <View className="px-4 pb-2">
          <Tabs<Tab>
            testID="search-kind"
            items={[
              { value: "all", label: t("kinds.all") },
              ...tabs.map((each) => ({
                value: kindSlug(each),
                label: t(`kinds.${kindSlug(each)}`),
              })),
            ]}
            value={kind === undefined ? "all" : kindSlug(kind)}
            onChange={(value) =>
              onKind(value === "all" ? undefined : tabs.find((each) => kindSlug(each) === value))
            }
          />
        </View>
      )}
      {results.isPending ? (
        <Spinner />
      ) : results.isError ? (
        <View className="px-4">
          <ErrorNote>{errorMessage(results.error, { namespace: "search" })}</ErrorNote>
        </View>
      ) : (
        <FlashList
          testID="search-results"
          data={hits}
          keyExtractor={(hit) => `${hit.kind}:${hit.id}`}
          keyboardShouldPersistTaps="handled"
          renderItem={({ item }) => <HitRow hit={item} onPress={() => openHit(item, onSearch)} />}
          onEndReached={() => {
            if (results.hasNextPage && !results.isFetchingNextPage) void results.fetchNextPage();
          }}
          ListFooterComponent={results.isFetchingNextPage ? <Spinner /> : null}
          ListEmptyComponent={
            <EmptyState
              emoji="🔍"
              title={t("empty.title", { query: asked.query })}
              body={`${t("empty.tipSpelling")}\n${t("empty.tipFewer")}`}
            />
          }
        />
      )}
    </View>
  );
}
