import { FlashList } from "@shopify/flash-list";
import { formatCount } from "@tokuly/core/video";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { EmptyState, Spinner } from "#/components/page";
import { useSubscriptions } from "./data";
import { ChannelAvatar, openChannel } from "./video-card";

/** Every channel the viewer subscribes to; a tap opens its page. */
export function SubscriptionsScreen() {
  const { t, i18n } = useTranslation("video");
  const subscriptions = useSubscriptions();
  const rows = subscriptions.data?.pages.flatMap((page) => page.subscriptions) ?? [];
  return (
    <View className="flex-1 bg-bg" testID="video-subscriptions">
      <FlashList
        data={rows}
        keyExtractor={(row) => row.channel?.id ?? ""}
        onEndReached={() => subscriptions.hasNextPage && void subscriptions.fetchNextPage()}
        ListEmptyComponent={
          subscriptions.isPending ? (
            <Spinner />
          ) : (
            <EmptyState
              emoji="📺"
              title={t("subscriptions.empty")}
              body={t("subscriptions.emptyBody")}
            />
          )
        }
        renderItem={({ item }) =>
          item.channel ? (
            <Pressable
              testID={`subscription-${item.channel.handle}`}
              accessibilityRole="link"
              onPress={() => item.channel && openChannel(item.channel)}
              className="flex-row items-center gap-3 px-4 py-3"
            >
              <ChannelAvatar channel={item.channel} size="md" />
              <View className="flex-1">
                <Text numberOfLines={1} className="font-black text-ink">
                  {item.channel.name}
                </Text>
                <Text className="text-xs text-ink-soft">
                  {t("channel.subscribers", {
                    count: Number(item.channel.subscriberCount),
                    formatted: formatCount(i18n.language, item.channel.subscriberCount),
                  })}
                </Text>
              </View>
            </Pressable>
          ) : null
        }
      />
    </View>
  );
}
