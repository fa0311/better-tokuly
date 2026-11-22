import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { toDate, type video } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, ListRow, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { channelClient, nextCursor, videoKeys } from "../api";

/** For the channel's owner: who subscribes, newest first. */
export function ChannelSubscribers({ channel }: { channel: video.Channel }) {
  const { t, i18n } = useTranslation("video");
  const channelId = channel.summary?.id ?? "";
  const query = useInfiniteQuery({
    queryKey: videoKeys.subscribers(channelId),
    queryFn: ({ pageParam }) =>
      channelClient().listSubscribers({ channelId, page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  const subscribers = query.data?.pages.flatMap((page) => page.subscribers) ?? [];

  if (query.isPending) return <Skeleton className="h-40 rounded-card" />;
  if (query.isError) return <EmptyState emoji="😵‍💫" title={errorMessage(query.error)} />;
  if (subscribers.length === 0)
    return (
      <EmptyState
        emoji="🫶"
        title={t("channel.noSubscribers")}
        body={t("channel.noSubscribersBody")}
      />
    );
  return (
    <section className="mx-auto max-w-2xl">
      <p className="px-1 pb-2 text-sm font-bold text-ink-soft">{t("channel.subscribersLead")}</p>
      <div className="sticker rounded-card bg-surface p-2">
        {subscribers.map(
          (subscriber) =>
            subscriber.user && (
              <ListRow
                key={subscriber.user.id}
                leading={
                  <Link to="/u/$handle" params={{ handle: subscriber.user.handle }}>
                    <Avatar user={subscriber.user} outlined />
                  </Link>
                }
                title={
                  <Link to="/u/$handle" params={{ handle: subscriber.user.handle }}>
                    <EmojiText text={subscriber.user.displayName} />
                  </Link>
                }
                subtitle={`@${subscriber.user.handle}`}
                trailing={
                  <span className="shrink-0 text-xs font-bold text-ink-faint">
                    {toDate(subscriber.subscribedAt)?.toLocaleDateString(i18n.language)}
                  </span>
                }
              />
            ),
        )}
        <LoadMore query={query} />
      </div>
    </section>
  );
}
