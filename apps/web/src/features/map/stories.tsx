import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type feed, toMillis } from "@tokuly/api";
import { ChevronLeft, ChevronRight, Pause, Play } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { useOpenPath } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { compactAge } from "#/lib/format";
import { useMe } from "#/stores/session";
import { useFlyTo } from "./camera";
import { mapKeys } from "./data";
import { MapPin } from "./map-view";
import { MapSheet } from "./sheet";
import { mapStoryClient, useStoryTray } from "./stories-api";
import { useMapUi } from "./ui-store";

/** How long one story item stays up before the reel moves on. */
const ITEM_MS = 6000;

/** The row of friends with recent located activity: tap a face to play their reel along the map. */
export function StoryTray() {
  const { t } = useTranslation("map");
  const { data } = useStoryTray();
  const open = useMapUi((state) => state.open);
  if (!data?.entries.length) return null;
  return (
    <ul
      className="scrollbar-none pointer-events-auto -mx-4 flex gap-2.5 overflow-x-auto px-4 py-1"
      data-testid="story-tray"
      aria-label={t("stories.title")}
    >
      {data.entries.map((entry) => {
        const user = entry.user;
        if (!user) return null;
        const unseen = entry.unseenCount > 0;
        return (
          <li key={user.id} className="shrink-0">
            <button
              type="button"
              data-unseen={unseen || undefined}
              onClick={() => open({ kind: "story", userId: user.id })}
              className="pressable flex w-14 shrink-0 flex-col items-center gap-0.5"
              aria-label={t("stories.play", { name: user.displayName, count: entry.itemCount })}
            >
              <span
                className={cn(
                  "rounded-[1.1rem] p-[3px]",
                  unseen ? "bg-linear-to-br from-brand via-pink to-sun" : "bg-line",
                )}
              >
                <span className="block rounded-[0.95rem] bg-surface p-[2px]">
                  <Avatar user={user} size="md" />
                </span>
              </span>
              <span className="glass w-full truncate rounded-full px-1 text-center text-[10px] font-black">
                <EmojiText text={user.displayName} />
              </span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}

/** One person's reel: each item in turn, with the map flying to where it happened. */
export function StoryPanel({ userId }: { userId: string }) {
  const { t } = useTranslation("map");
  const queryClient = useQueryClient();
  const myId = useMe()?.user?.id;
  const flyTo = useFlyTo();
  const openPath = useOpenPath();
  const { stack, back, close, open } = useMapUi();
  const tray = useStoryTray();
  const { data: reel, isPending } = useQuery({
    queryKey: mapKeys.storyReel(userId),
    queryFn: async () => {
      const response = await mapStoryClient().listMapStories({ userId });
      return (
        (userId === myId
          ? response.myReel
          : response.reels.find((entry) => entry.user?.id === userId)) ?? null
      );
    },
    staleTime: 30_000,
  });
  const [index, setIndex] = useState<number | null>(null);
  const [paused, setPaused] = useState(false);
  const seen = useRef(new Set<string>());

  const items = reel?.items ?? [];
  // Start at the first item not watched yet.
  const start = Math.max(
    0,
    items.findIndex((item) => !item.seen),
  );
  const current = Math.min(index ?? start, Math.max(0, items.length - 1));
  const item: feed.MapStoryItem | undefined = items[current];
  const activity = item?.activity;

  const leave = () => {
    void queryClient.invalidateQueries({ queryKey: mapKeys.storyTray });
    // On to the next friend with something new, like a story tray does; otherwise done.
    const entries = tray.data?.entries ?? [];
    const position = entries.findIndex((entry) => entry.user?.id === userId);
    const next = entries.slice(position + 1).find((entry) => entry.unseenCount > 0 && entry.user);
    if (next?.user) open({ kind: "story", userId: next.user.id });
    else if (stack.length > 1) back();
    else close();
  };
  const go = (step: number) => {
    const target = current + step;
    if (target < 0) return;
    if (target >= items.length) leave();
    else setIndex(target);
  };

  // Each item: move the map there and remember that it was watched.
  const activityId = activity?.id;
  const lat = activity?.location?.lat;
  const lng = activity?.location?.lng;
  useEffect(() => {
    if (!activityId) return;
    if (lat !== undefined && lng !== undefined) flyTo(lng, lat, 14.5);
    if (userId === myId || seen.current.has(activityId)) return;
    seen.current.add(activityId);
    void mapStoryClient()
      .markMapStorySeen({ activityIds: [activityId] })
      .catch(() => undefined);
  }, [activityId, lat, lng, userId, myId]);

  useEffect(() => {
    if (paused || !activityId) return;
    const timer = setTimeout(() => go(1), ITEM_MS);
    return () => clearTimeout(timer);
  }, [activityId, paused, current]);

  const user = reel?.user;
  return (
    <MapSheet
      testId="story-panel"
      title={user?.displayName ?? t("stories.title")}
      subtitle={t("stories.subtitle")}
      leading={user && <Avatar user={user} size="md" outlined />}
      onBack={stack.length > 1 ? back : undefined}
      onClose={close}
    >
      {isPending ? (
        <Skeleton className="h-32" />
      ) : !activity ? (
        <p className="py-8 text-center text-sm text-ink-soft">{t("stories.empty")}</p>
      ) : (
        <div>
          <div className="flex gap-1" aria-hidden>
            {items.map((entry, position) => (
              <span
                key={entry.activity?.id ?? position}
                className="h-1.5 flex-1 overflow-hidden rounded-full bg-surface-3"
              >
                <span
                  key={`${position}-${current}-${paused}`}
                  className={cn(
                    "block h-full rounded-full bg-brand",
                    position < current && "w-full",
                    position > current && "w-0",
                  )}
                  style={
                    position === current
                      ? {
                          width: paused ? "50%" : "100%",
                          animation: paused ? undefined : `story-progress ${ITEM_MS}ms linear both`,
                        }
                      : undefined
                  }
                />
              </span>
            ))}
          </div>
          <style>{"@keyframes story-progress { from { width: 0% } to { width: 100% } }"}</style>

          <article className="mt-3" data-testid="story-item" aria-live="polite">
            <p className="flex items-center gap-2 text-[11px] font-black text-ink-soft">
              <span className="rounded-full bg-brand-soft px-2 py-0.5 text-brand">
                {activity.kindLabel}
              </span>
              <span>{compactAge(toMillis(activity.occurredAt))}</span>
              <span className="ml-auto tabular-nums">
                {current + 1} / {items.length}
              </span>
            </p>
            {activity.imageUrl && (
              <img
                src={activity.imageUrl}
                alt=""
                className="mt-2 aspect-video w-full rounded-2xl object-cover"
              />
            )}
            <p className="mt-2 text-base leading-snug font-black">{activity.summary}</p>
            {activity.withUsers.length > 0 && (
              <p className="mt-2 flex items-center gap-1.5 text-xs font-bold text-ink-soft">
                <span className="flex -space-x-1.5">
                  {activity.withUsers.slice(0, 4).map((other) => (
                    <Avatar key={other.id} user={other} size="xs" outlined />
                  ))}
                </span>
                {t("stories.with", {
                  names: activity.withUsers.map((other) => other.displayName).join("、"),
                })}
              </p>
            )}
            {activity.path && (
              <button
                type="button"
                onClick={() => openPath(activity.path)}
                className="mt-2 inline-block text-sm font-bold text-brand"
              >
                {t("stories.open")} →
              </button>
            )}
          </article>

          <div className="mt-3 flex items-center justify-between">
            <button
              type="button"
              onClick={() => go(-1)}
              disabled={current === 0}
              aria-label={t("stories.previous")}
              className="pressable grid size-11 place-items-center rounded-full bg-surface-2 disabled:opacity-40"
            >
              <ChevronLeft className="size-5" />
            </button>
            <button
              type="button"
              onClick={() => setPaused((value) => !value)}
              aria-label={paused ? t("stories.resume") : t("stories.pause")}
              className="pressable grid size-11 place-items-center rounded-full bg-surface-2"
            >
              {paused ? <Play className="size-5" /> : <Pause className="size-5" />}
            </button>
            <button
              type="button"
              onClick={() => go(1)}
              aria-label={t("stories.next")}
              className="pressable grid size-11 place-items-center rounded-full bg-brand text-on-brand"
            >
              <ChevronRight className="size-5" />
            </button>
          </div>

          {activity.location && user && (
            <MapPin lng={activity.location.lng} lat={activity.location.lat} zIndex={40}>
              <div className="flex flex-col items-center" data-testid="story-pin">
                <span className="animate-pop-in rounded-[1.25rem] bg-linear-to-br from-brand via-pink to-sun p-[3px]">
                  <span className="block rounded-[1.1rem] bg-surface p-[2px]">
                    <Avatar user={user} size="lg" className="block" />
                  </span>
                </span>
                <span className="h-2.5 w-[3px] bg-ink" aria-hidden />
                <span className="max-w-44 truncate rounded-full bg-ink px-2.5 py-0.5 text-[11px] font-black text-surface">
                  {activity.kindLabel}
                </span>
              </div>
            </MapPin>
          )}
        </div>
      )}
    </MapSheet>
  );
}
