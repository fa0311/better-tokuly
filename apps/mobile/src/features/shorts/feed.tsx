import { type FeedContext, type FeedEntry, playbackSource, slideLoad } from "@tokuly/core/shorts";
import { router, useFocusEffect } from "expo-router";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AppState, FlatList, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { useShortList } from "./data";
import { ShortSlide } from "./slide";

/**
 * A swipe feed: one short a screen, the one on screen playing and the next
 * loading ahead (`slideLoad`). The list behind it is a feed context, so a
 * short tapped in a grid opens inside the list already loaded.
 */
export function ShortsFeed({ context, startId }: { context: FeedContext; startId?: string }) {
  const { t } = useTranslation("shorts");
  const list = useShortList(context);
  const entries: FeedEntry[] = list.data?.pages.flatMap((page) => page.entries) ?? [];
  if (list.isPending) return <Spinner />;
  if (list.isError)
    return (
      <View className="flex-1 items-center justify-center bg-bg">
        <EmptyState emoji="😵" title={t("feed.gone")} body={t("feed.goneBody")} />
        <Button onPress={() => void list.refetch()}>{t("feed.retry")}</Button>
      </View>
    );
  if (entries.length === 0)
    return (
      <View className="flex-1 items-center justify-center gap-3 bg-bg" testID="shorts-empty">
        <EmptyState emoji="🎬" title={t("feed.empty")} body={t("feed.emptyBody")} />
        <Button onPress={() => router.push("/shorts/new")}>{t("feed.post")}</Button>
      </View>
    );
  const start = startId
    ? entries.findIndex((entry) => entry.short.id === startId || entry.short.shortId === startId)
    : 0;
  return (
    <Pager
      entries={entries}
      context={context}
      start={Math.max(0, start)}
      onEnd={() => {
        if (list.hasNextPage && !list.isFetchingNextPage) void list.fetchNextPage();
      }}
    />
  );
}

function Pager({
  entries,
  context,
  start,
  onEnd,
}: {
  entries: FeedEntry[];
  context: FeedContext;
  start: number;
  onEnd(): void;
}) {
  const [height, setHeight] = useState(0);
  const [current, setCurrent] = useState(start);
  const [inFront, setInFront] = useState(AppState.currentState !== "background");
  const [focused, setFocused] = useState(true);
  const source = playbackSource(context);

  // Nothing plays while the app is in the background, or another screen is in front.
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setInFront(state === "active"),
    );
    return () => subscription.remove();
  }, []);
  useFocusEffect(
    useCallback(() => {
      setFocused(true);
      return () => setFocused(false);
    }, []),
  );

  const onViewable = useRef(
    ({ viewableItems }: { viewableItems: { index?: number | null; isViewable: boolean }[] }) => {
      const shown = viewableItems.find((item) => item.isViewable)?.index;
      if (typeof shown === "number") setCurrent(shown);
    },
  ).current;
  const watching = inFront && focused;

  // Two shorts before the end, the next page is asked for (whatever the list's own scrolling says).
  const ahead = useRef(onEnd);
  ahead.current = onEnd;
  const nearEnd = current >= entries.length - 2;
  useEffect(() => {
    if (nearEnd) ahead.current();
  }, [nearEnd]);

  return (
    <View
      testID="shorts-feed"
      className="flex-1 bg-black"
      onLayout={(event) => setHeight(Math.round(event.nativeEvent.layout.height))}
    >
      {height > 0 && (
        <FlatList
          data={entries}
          keyExtractor={(entry, index) => `${entry.short.id}:${index}`}
          pagingEnabled
          showsVerticalScrollIndicator={false}
          initialScrollIndex={start}
          getItemLayout={(_, index) => ({ length: height, offset: height * index, index })}
          windowSize={5}
          initialNumToRender={3}
          maxToRenderPerBatch={3}
          viewabilityConfig={{ itemVisiblePercentThreshold: 80 }}
          onViewableItemsChanged={onViewable}
          onEndReachedThreshold={2}
          onEndReached={onEnd}
          renderItem={({ item, index }) => {
            const load = slideLoad(index, current);
            return (
              <ShortSlide
                entry={item}
                index={index}
                height={height}
                source={source}
                load={load === "playing" && !watching ? "preloading" : load}
              />
            );
          }}
        />
      )}
    </View>
  );
}
