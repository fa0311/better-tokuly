import { FlashList } from "@shopify/flash-list";
import { video } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { Alert, Switch, Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { useForget, useHistory, useHistorySettings } from "./data";
import { VideoCard } from "./video-card";

const { TrafficSource } = video;

/**
 * What the viewer watched, most recent first, each card showing how far
 * they got. Holding a card takes it out; recording can be paused, and the
 * history cleared (with every resume position).
 */
export function HistoryScreen() {
  const { t } = useTranslation("video");
  const history = useHistory();
  const forget = useForget();
  const settings = useHistorySettings();
  const paused = history.data?.pages[0]?.paused ?? false;
  const items = history.data?.pages.flatMap((page) => page.items) ?? [];
  const clear = () =>
    Alert.alert(t("history.confirmClear"), t("history.confirmClearBody"), [
      { text: t("actions.cancel"), style: "cancel" },
      { text: t("history.clear"), style: "destructive", onPress: () => void settings.clear() },
    ]);
  return (
    <View className="flex-1 bg-bg" testID="video-history">
      <FlashList
        data={items}
        keyExtractor={(item) => item.video?.id ?? ""}
        onEndReached={() => history.hasNextPage && void history.fetchNextPage()}
        ListHeaderComponent={
          <View className="gap-3 px-4 pt-4 pb-3">
            <View className="flex-row items-center gap-3 rounded-card bg-surface px-4 py-3">
              <View className="flex-1">
                <Text className="font-black text-ink">{t("history.pause")}</Text>
                <Text className="text-xs text-ink-soft">
                  {paused ? t("history.pausedHint") : t("history.pauseHint")}
                </Text>
              </View>
              <Switch
                testID="history-pause"
                value={paused}
                onValueChange={(value) => void settings.setPaused(value)}
              />
            </View>
            {items.length > 0 ? (
              <Button size="sm" variant="secondary" onPress={clear}>
                {t("history.clear")}
              </Button>
            ) : null}
          </View>
        }
        ListEmptyComponent={
          history.isPending ? (
            <Spinner />
          ) : paused ? (
            <EmptyState
              emoji="⏸️"
              title={t("history.emptyPaused")}
              body={t("history.emptyPausedBody")}
            />
          ) : (
            <EmptyState emoji="🕰️" title={t("history.empty")} body={t("history.emptyBody")} />
          )
        }
        renderItem={({ item }) =>
          item.video ? (
            <VideoCard
              summary={item.video}
              source={TrafficSource.HISTORY}
              menu={[
                {
                  key: "forget",
                  label: t("history.remove"),
                  destructive: true,
                  onPress: () => item.video && void forget(item.video.id),
                },
              ]}
            />
          ) : null
        }
      />
    </View>
  );
}
