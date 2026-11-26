import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type live, toMillis } from "@tokuly/api";
import { tierByLevel } from "@tokuly/core/live";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Coins } from "#/components/ui/coins";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Tabs } from "#/components/ui/tabs";
import { EmojiText } from "#/features/emoji";
import { useMyChannels } from "#/features/video";
import { BalancePill } from "#/features/wallet";
import { giftClient, liveKeys, nextCursor, useGiftTiers } from "../api";
import { tierStyle } from "../model";
import { ChannelFace, formatMoment } from "../parts";

type Tab = "sent" | "received";

/** One gift of a history list: its tier's colour, the amount, the words, where and when. */
function GiftRow({
  gift,
  tiers,
  received,
}: {
  gift: live.Gift;
  tiers: live.GiftTier[];
  received: boolean;
}) {
  const { t, i18n } = useTranslation("live");
  const style = tierStyle(tierByLevel(tiers, gift.tier));
  const title = gift.streamTitle || t("giftHistory.streamGone");
  return (
    <li className="flex items-start gap-3 rounded-2xl px-2 py-3" data-testid="gift-row">
      {received ? (
        <Avatar user={gift.sender} size="md" />
      ) : (
        gift.channel && <ChannelFace channel={gift.channel} size="md" />
      )}
      <div className="min-w-0 flex-1">
        <p className="truncate font-black">
          <EmojiText
            text={
              received
                ? (gift.sender?.displayName ?? t("giftHistory.senderGone"))
                : (gift.channel?.name ?? "")
            }
          />
        </p>
        {gift.message && (
          <p className="mt-0.5 text-sm leading-snug break-words">
            <EmojiText text={gift.message} />
          </p>
        )}
        <p className="mt-0.5 truncate text-xs font-bold text-ink-faint">
          {gift.streamTitle ? (
            <Link
              to="/live/$streamId"
              params={{ streamId: gift.streamId }}
              className="hover:underline"
            >
              <EmojiText text={title} />
            </Link>
          ) : (
            title
          )}
          {gift.createdAt && ` · ${formatMoment(toMillis(gift.createdAt), i18n.language)}`}
        </p>
      </div>
      <div className="flex shrink-0 flex-col items-end gap-1">
        <span
          className="inline-flex h-7 items-center rounded-full border-2 border-ink px-2.5"
          style={style}
          title={t("giftHistory.tier", { level: gift.tier })}
        >
          <Coins amount={gift.coins} size="sm" />
        </span>
        {received && gift.netCoins > 0n && (
          <span className="text-xs font-bold text-ink-faint">
            {t("giftHistory.net", { coins: gift.netCoins.toLocaleString(i18n.language) })}
          </span>
        )}
      </div>
    </li>
  );
}

function GiftList({
  query,
  received,
  emptyTitle,
  emptyBody,
}: {
  query: {
    data?: { pages: { gifts: live.Gift[] }[] };
    isPending: boolean;
    hasNextPage: boolean;
    isFetchingNextPage: boolean;
    fetchNextPage(): unknown;
  };
  received: boolean;
  emptyTitle: string;
  emptyBody: string;
}) {
  const tiers = useGiftTiers().data?.tiers ?? [];
  const gifts = query.data?.pages.flatMap((page) => page.gifts) ?? [];
  if (query.isPending)
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-16 w-full" />
        <Skeleton className="h-16 w-full" />
      </div>
    );
  if (gifts.length === 0) return <EmptyState emoji="🎁" title={emptyTitle} body={emptyBody} />;
  return (
    <>
      <ul className="flex flex-col divide-y-2 divide-line">
        {gifts.map((gift) => (
          <GiftRow key={gift.id} gift={gift} tiers={tiers} received={received} />
        ))}
      </ul>
      <LoadMore query={query} />
    </>
  );
}

function SentGifts() {
  const { t } = useTranslation("live");
  const query = useInfiniteQuery({
    queryKey: liveKeys.sentGifts,
    queryFn: ({ pageParam }) =>
      giftClient().listSentGifts({ page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  return (
    <GiftList
      query={query}
      received={false}
      emptyTitle={t("giftHistory.emptySentTitle")}
      emptyBody={t("giftHistory.emptySentBody")}
    />
  );
}

/** Gifts received on one of the caller's channels (optionally one stream), with what they add up to. */
export function ReceivedGifts({
  channelId,
  streamId = "",
}: {
  channelId: string;
  streamId?: string;
}) {
  const { t, i18n } = useTranslation("live");
  const query = useInfiniteQuery({
    queryKey: liveKeys.receivedGifts(channelId, streamId),
    queryFn: ({ pageParam }) =>
      giftClient().listReceivedGifts({
        channelId,
        streamId,
        page: { cursor: pageParam, limit: 30 },
      }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    enabled: channelId !== "",
  });
  const first = query.data?.pages[0];
  return (
    <>
      {first && first.totalCoins > 0n && (
        <div className="mb-3 flex flex-wrap items-center gap-x-6 gap-y-1 rounded-2xl bg-surface-2 px-4 py-3">
          <span className="text-sm font-bold text-ink-soft">{t("giftHistory.total")}</span>
          <Coins amount={first.totalCoins} />
          <span className="text-sm font-bold text-ink-soft">
            {t("giftHistory.totalNet", {
              coins: first.totalNetCoins.toLocaleString(i18n.language),
            })}
          </span>
        </div>
      )}
      <GiftList
        query={query}
        received
        emptyTitle={t("giftHistory.emptyReceivedTitle")}
        emptyBody={t("giftHistory.emptyReceivedBody")}
      />
    </>
  );
}

function ReceivedTab() {
  const { t } = useTranslation("live");
  const channels = useMyChannels().data?.channels ?? [];
  const [chosen, setChosen] = useState("");
  const channelId = chosen || channels[0]?.summary?.id || "";
  if (channels.length === 0)
    return (
      <EmptyState
        emoji="📺"
        title={t("giftHistory.noChannelTitle")}
        body={t("giftHistory.noChannelBody")}
      />
    );
  return (
    <>
      {channels.length > 1 && (
        <SelectField
          className="mb-3"
          label={t("giftHistory.channel")}
          value={channelId}
          onChange={setChosen}
          options={channels.map((channel) => ({
            value: channel.summary?.id ?? "",
            label: channel.summary?.name ?? "",
          }))}
        />
      )}
      <ReceivedGifts channelId={channelId} />
    </>
  );
}

/** `/live/gifts`: the gifts the person sent and the ones their channels received. */
export function GiftHistoryScreen() {
  const { t } = useTranslation("live");
  const [tab, setTab] = useState<Tab>("sent");
  return (
    <Page title={t("giftHistory.title")} back="/live" actions={<BalancePill />}>
      <Tabs<Tab>
        className="mb-4"
        items={[
          { value: "sent", label: t("giftHistory.sent") },
          { value: "received", label: t("giftHistory.received") },
        ]}
        value={tab}
        onChange={setTab}
      />
      {tab === "sent" ? <SentGifts /> : <ReceivedTab />}
    </Page>
  );
}
