import { useMutation, useQueryClient } from "@tanstack/react-query";
import { common, geo } from "@tokuly/api";
import { compactDuration, formatDistance } from "@tokuly/core/format";
import {
  LABELS,
  labelMeta,
  placeKeys,
  shownSuggestions,
  suggestionMark,
} from "@tokuly/core/places";
import { router } from "expo-router";
import { Plus } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Page, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { TextField } from "#/components/text-field";
import { GazetteerCredit } from "#/features/history/parts";
import { useMe } from "#/features/session/store";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import {
  asAccount,
  NS,
  placeClient,
  useDismissed,
  useMyPlaces,
  usePlaceSearch,
  useSavedPlaces,
  useSuggestions,
} from "./data";
import { PlaceRow } from "./parts";

type Tab = "mine" | "saved" | "search";

const openWeb = (path: string) => router.push(webScreenHref(path) as never);

/** `/places`: the own places, the saved ones, and finding more. */
export function PlacesScreen() {
  const { t } = useTranslation(NS);
  const [tab, setTab] = useState<Tab>("mine");
  return (
    <Page testID="places">
      <Tabs<Tab>
        testID="places-tab"
        items={[
          { value: "mine", label: t("tabs.mine") },
          { value: "saved", label: t("tabs.saved") },
          { value: "search", label: t("tabs.search") },
        ]}
        value={tab}
        onChange={setTab}
      />
      {tab === "mine" && <MyPlaces />}
      {tab === "saved" && <SavedPlaces />}
      {tab === "search" && <PlaceSearch />}
    </Page>
  );
}

function MyPlaces() {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const query = useMyPlaces();
  if (query.isPending) return <Spinner />;
  if (query.isError) return <ErrorNote>{errorMessage(query.error, { namespace: NS })}</ErrorNote>;
  const places = query.data.places;
  const custom = places.filter(
    (place) => place.label === geo.PlaceLabel.CUSTOM || place.label === geo.PlaceLabel.UNSPECIFIED,
  );
  return (
    <View className="gap-5">
      <Suggestions />
      <View className="gap-3">
        <View className="px-1">
          <Text className="text-lg font-black text-ink">{t("mine.usual")}</Text>
          <Text className="text-sm text-ink-soft">{t("mine.usualBody")}</Text>
        </View>
        {([geo.PlaceLabel.HOME, geo.PlaceLabel.SCHOOL, geo.PlaceLabel.WORK] as const).map(
          (label) => {
            const place = places.find((candidate) => candidate.label === label);
            const meta = LABELS[label];
            return place ? (
              <PlaceRow key={label} place={place} />
            ) : (
              <Pressable
                key={label}
                testID={`places-add-${meta.key}`}
                accessibilityRole="button"
                onPress={() => openWeb(`/places/new?label=${meta.key}`)}
                className="flex-row items-center gap-3 rounded-card border-[2.5px] border-dashed border-line bg-surface p-3"
              >
                <View className="size-12 items-center justify-center rounded-2xl bg-surface-2 opacity-60">
                  <Text className="text-2xl">{meta.emoji}</Text>
                </View>
                <Text className="flex-1 font-bold text-ink-soft">{t(`mine.add.${meta.key}`)}</Text>
                <Plus size={20} color={colors["ink-soft"]} />
              </Pressable>
            );
          },
        )}
      </View>
      <View className="gap-3">
        <View className="flex-row items-center justify-between px-1">
          <Text className="text-lg font-black text-ink">{t("mine.custom")}</Text>
          <Button
            testID="places-add"
            size="sm"
            icon={<Plus size={16} color={colors["on-brand"]} />}
            onPress={() => openWeb("/places/new")}
          >
            {t("actions.add")}
          </Button>
        </View>
        {custom.length === 0 ? (
          <EmptyState emoji="📍" title={t("mine.emptyTitle")} body={t("mine.emptyBody")} />
        ) : (
          custom.map((place) => <PlaceRow key={place.id} place={place} />)
        )}
      </View>
    </View>
  );
}

/** "よく行く場所を登録しますか?": spots the server noticed the person keeps staying at. */
function Suggestions() {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const userId = useMe()?.user?.id ?? "";
  const { data } = useSuggestions();
  const { dismissed, dismiss } = useDismissed(userId);
  const all = data?.suggestions ?? [];
  const accept = useMutation({
    mutationFn: (suggestion: geo.PlaceSuggestion) =>
      asAccount(() =>
        placeClient().createPlace({
          name: suggestion.suggestedName || t(`labels.${labelMeta(suggestion.suggestedLabel).key}`),
          label: suggestion.suggestedLabel || geo.PlaceLabel.CUSTOM,
          center: suggestion.center,
          visibility: common.Visibility.PRIVATE,
        }),
      ),
    onSuccess: (answer, suggestion) => {
      if (!answer) return;
      // It is a place now: not offered again while the server still lists it.
      dismiss(suggestion, all);
      Alert.alert(t("suggestions.accepted", { name: answer.place?.name ?? "" }));
      void queryClient.invalidateQueries({ queryKey: placeKeys.all });
    },
    onError: (error) => Alert.alert(errorMessage(error, { namespace: NS })),
  });
  const shown = shownSuggestions(userId, all, dismissed);
  if (shown.length === 0) return null;
  return (
    <View className="gap-3">
      <Text className="px-1 text-lg font-black text-ink">{t("suggestions.title")}</Text>
      {shown.map((suggestion) => {
        const meta = labelMeta(suggestion.suggestedLabel);
        const name = suggestion.suggestedName || t(`labels.${meta.key}`);
        const mark = suggestionMark(userId, suggestion);
        return (
          <View
            key={mark}
            testID="places-suggestion"
            className="gap-3 rounded-card bg-sun p-4"
            style={sticker(colors)}
          >
            <View className="flex-row items-center gap-3">
              <Text className="text-3xl">{meta.emoji}</Text>
              <View className="flex-1">
                <Text className="font-black" style={{ color: INK_ON_BRIGHT }}>
                  {t("suggestions.question", { name })}
                </Text>
                <Text className="text-sm font-bold opacity-75" style={{ color: INK_ON_BRIGHT }}>
                  {t("suggestions.stats", {
                    count: suggestion.visitCount,
                    duration: compactDuration(
                      i18n.language,
                      Number(suggestion.totalSeconds) * 1000,
                    ),
                  })}
                </Text>
              </View>
            </View>
            <View className="flex-row justify-end gap-2">
              <Button
                testID="places-suggestion-dismiss"
                variant="ghost"
                size="sm"
                onPress={() => dismiss(suggestion, all)}
              >
                {t("suggestions.dismiss")}
              </Button>
              <Button
                testID="places-suggestion-accept"
                variant="secondary"
                size="sm"
                loading={accept.isPending && accept.variables === suggestion}
                onPress={() => accept.mutate(suggestion)}
              >
                {t("suggestions.accept")}
              </Button>
            </View>
          </View>
        );
      })}
    </View>
  );
}

function SavedPlaces() {
  const { t } = useTranslation(NS);
  const query = useSavedPlaces();
  if (query.isPending) return <Spinner />;
  if (query.isError) return <ErrorNote>{errorMessage(query.error, { namespace: NS })}</ErrorNote>;
  const places = query.data.pages.flatMap((page) => page.places);
  if (places.length === 0)
    return <EmptyState emoji="🔖" title={t("saved.emptyTitle")} body={t("saved.emptyBody")} />;
  return (
    <View className="gap-3">
      {places.map((place) => (
        <PlaceRow key={place.id} place={place} subtitle={place.address} />
      ))}
      {query.hasNextPage && (
        <Button
          variant="ghost"
          loading={query.isFetchingNextPage}
          onPress={() => void query.fetchNextPage()}
        >
          {t("common:actions.more")}
        </Button>
      )}
    </View>
  );
}

/**
 * Finding places by name: the caller's, friends' and public places, and
 * spots from the place-name data. The phone's position is not sent.
 */
function PlaceSearch() {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const [text, setText] = useState("");
  const [query, setQuery] = useState("");
  // The search follows the typing with a short delay.
  useEffect(() => {
    const timer = setTimeout(() => setQuery(text.trim()), 350);
    return () => clearTimeout(timer);
  }, [text]);
  const search = usePlaceSearch(query);
  const results = search.data?.results ?? [];
  return (
    <View className="gap-3">
      <TextField
        testID="places-search"
        label={t("search.label")}
        placeholder={t("search.placeholder")}
        value={text}
        maxLength={100}
        onChangeText={setText}
        returnKeyType="search"
      />
      {query === "" ? (
        <EmptyState emoji="🗺️" title={t("search.hintTitle")} body={t("search.hintBody")} />
      ) : search.isPending ? (
        <Spinner />
      ) : search.isError ? (
        <ErrorNote>{errorMessage(search.error, { namespace: NS })}</ErrorNote>
      ) : results.length === 0 ? (
        <EmptyState emoji="🔍" title={t("search.emptyTitle")} body={t("search.emptyBody")} />
      ) : (
        <>
          {results.map((result, index) => {
            const distance =
              result.distanceMeters !== undefined ? formatDistance(result.distanceMeters) : "";
            if (result.place)
              return (
                <PlaceRow
                  key={result.place.id}
                  place={result.place}
                  subtitle={[result.address || result.place.address, distance]
                    .filter(Boolean)
                    .join(" · ")}
                  trailing={
                    result.friendsVisited.length > 0 ? (
                      <View
                        className="flex-row"
                        accessibilityLabel={t("search.friendsVisited", {
                          count: result.friendsVisited.length,
                        })}
                      >
                        {result.friendsVisited.slice(0, 3).map((friend) => (
                          <Avatar key={friend.id} user={friend} size="xs" />
                        ))}
                      </View>
                    ) : undefined
                  }
                />
              );
            // A spot from the place-name data: not a Tokuly place (yet); it is made on the web.
            const position = result.position;
            return (
              <View
                key={`${result.name}:${index}`}
                testID="places-hit"
                className="gap-2 rounded-card border-2 border-line bg-surface p-3"
              >
                <View className="flex-row items-center gap-3">
                  <View className="size-12 items-center justify-center rounded-2xl bg-surface-2">
                    <Text className="text-2xl">🗾</Text>
                  </View>
                  <View className="min-w-0 flex-1">
                    <Text numberOfLines={1} className="font-black text-ink">
                      {result.name}
                    </Text>
                    <Text numberOfLines={1} className="text-sm text-ink-soft">
                      {[result.address, distance].filter(Boolean).join(" · ")}
                    </Text>
                  </View>
                </View>
                {position && (
                  <View className="flex-row justify-end">
                    <Button
                      size="sm"
                      variant="secondary"
                      icon={<Plus size={14} color={colors.ink} />}
                      onPress={() =>
                        openWeb(
                          `/places/new?lat=${position.lat}&lng=${position.lng}&name=${encodeURIComponent(result.name)}`,
                        )
                      }
                    >
                      {t("search.register")}
                    </Button>
                  </View>
                )}
              </View>
            );
          })}
          <GazetteerCredit />
        </>
      )}
    </View>
  );
}
