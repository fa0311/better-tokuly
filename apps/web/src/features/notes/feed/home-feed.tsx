import { feed } from "@tokuly/api";
import { type ReactNode, type Ref, useEffect, useImperativeHandle, useRef } from "react";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { type TimelineHandle, TimelineSkeleton } from "../timeline";
import { onTimelineHint } from "../timeline-hints";
import { ActivityCardView } from "./activity-card";
import { useHomeFeed } from "./api";

/** Whether the home feed holds anything a timeline of notes would not show: then it earns its own tab. */
export function hasMoreThanNotes(cards: feed.ActivityCard[]): boolean {
  return cards.some((card) => card.type !== feed.ActivityCardType.NOTE);
}

/**
 * The mixed home feed: what friends posted and did — notes, stories,
 * videos, bumps, new friends — as one list of cards, newest first.
 */
export function HomeFeed({ empty, ref }: { empty: ReactNode; ref?: Ref<TimelineHandle> }) {
  const { t } = useTranslation("notes");
  const query = useHomeFeed();
  const cards = query.data?.pages.flatMap((page) => page.cards) ?? [];
  const refetch = useRef(query.refetch);
  const pageCount = useRef(0);
  useEffect(() => {
    refetch.current = query.refetch;
    pageCount.current = query.data?.pages.length ?? 0;
  });

  useImperativeHandle(ref, () => ({
    refresh: async () => {
      await refetch.current();
    },
  }));

  // While only the first page is loaded, reading it again is cheap: do it when something new is hinted at.
  useEffect(
    () =>
      onTimelineHint(() => {
        if (pageCount.current === 1) void refetch.current();
      }),
    [],
  );

  if (query.isPending) return <TimelineSkeleton />;
  if (query.isError && cards.length === 0) {
    return (
      <EmptyState
        emoji="🌧️"
        title={errorMessage(query.error, { namespace: "notes" })}
        action={<Button onClick={() => void query.refetch()}>{t("timeline.retry")}</Button>}
      />
    );
  }
  if (cards.length === 0) return <>{empty}</>;
  return (
    <div className="flex flex-col gap-2.5" data-testid="home-feed">
      {cards.map(
        (card) => card.activity && <ActivityCardView key={card.activity.id} card={card} />,
      )}
      <LoadMore query={query} showEnd />
    </div>
  );
}
