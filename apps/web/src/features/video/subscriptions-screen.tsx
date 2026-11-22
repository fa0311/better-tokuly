import { useInfiniteQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { channelClient, nextCursor, videoKeys } from "./api";
import { ChannelRow } from "./channel/channel-row";
import { RecommendedChannels } from "./feeds-screen";

/** `/watch/channels`: the channels the viewer subscribes to — each with its notification level — and channels worth a look. */
export function SubscriptionsScreen() {
  const { t } = useTranslation("video");
  const subscriptions = useInfiniteQuery({
    queryKey: videoKeys.subscriptions,
    queryFn: ({ pageParam }) => channelClient().listSubscriptions({ page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  const rows = subscriptions.data?.pages.flatMap((page) => page.subscriptions) ?? [];

  return (
    <Page title={t("subscriptions.title")} back="/watch">
      {subscriptions.isPending ? (
        <div className="flex flex-col gap-2">
          {[0, 1, 2].map((row) => (
            <Skeleton key={row} className="h-16" />
          ))}
        </div>
      ) : subscriptions.isError ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(subscriptions.error)} />
      ) : rows.length === 0 ? (
        <EmptyState emoji="📺" title={t("subscriptions.empty")} body={t("subscriptions.emptyBody")} />
      ) : (
        <div className="sticker rounded-card bg-surface p-2">
          {rows.map((subscription) => subscription.channel && <ChannelRow key={subscription.channel.id} channel={subscription.channel} subscribed notificationLevel={subscription.notificationLevel} />)}
          <LoadMore query={subscriptions} />
        </div>
      )}
      <div className="mt-8">
        <RecommendedChannels />
      </div>
    </Page>
  );
}
