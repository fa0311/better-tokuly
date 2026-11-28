import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { errorReason } from "@tokuly/api";
import {
  activityMeta,
  buildTimeline,
  dayRange,
  decodePolyline,
  historyQueries,
  parseDay,
  shiftDay,
  toTrack,
  visitEmoji,
  visitName,
} from "@tokuly/core/history";
import { ChevronLeft, ChevronRight, EllipsisVertical, Trash2 } from "lucide-react";
import { useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { GazetteerCredit, toLatLng } from "#/features/places";
import { useAccountTimeZone, useAccountToday } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { compactDuration, formatDistance, kmh } from "#/lib/format";
import { useTranslation } from "#/lib/use-translation";
import { historyClient } from "./api";
import { DayMap, type DayStop } from "./day-map";
import { DeleteHistoryDialog } from "./delete-history";
import { DayMemoEditor } from "./memo";
import { VisitCard } from "./visit-card";

/** `/history/<YYYY-MM-DD>`: one day as a diary — where I was, how I got around, and what I want to remember. */
export function DayScreen({ date }: { date: string }) {
  const { t, i18n } = useTranslation("history");
  const navigate = useNavigate();
  const timeZone = useAccountTimeZone();
  const range = dayRange(date, timeZone);
  const today = useAccountToday();
  const [focus, setFocus] = useState<{ id: string; nonce: number } | null>(null);
  const [deleting, setDeleting] = useState(false);

  const day = useQuery({
    ...historyQueries.day(historyClient, date, timeZone),
    enabled: Boolean(range),
  });
  // The points of the day with their times, for the replay. When the store
  // that keeps them cannot be reached, the day is shown without it.
  const route = useQuery(historyQueries.route(historyClient, date, timeZone));

  if (!range) {
    return (
      <Page title={t("title")} back="/history">
        <EmptyState emoji="🗓️" title={t("day.invalid")} />
      </Page>
    );
  }

  const title = parseDay(date)!.toLocaleDateString(i18n.language, {
    month: "long",
    day: "numeric",
    weekday: "short",
  });
  const summary = day.data?.summary;
  const visits = day.data?.visits ?? [];
  const track = toTrack(route.data?.points ?? []);
  const path = decodePolyline(day.data?.pathPolyline ?? "");
  const timeline = buildTimeline(visits, track);
  // Visits are numbered in the order of the day, the same on the map and in the list.
  const orderOf = new Map(
    timeline
      .flatMap((item) => (item.kind === "visit" ? [item.visit.id] : []))
      .map((id, index) => [id, index + 1]),
  );
  const stops: DayStop[] = visits.flatMap((visit) =>
    visit.center
      ? [
          {
            id: visit.id,
            position: toLatLng(visit.center)!,
            order: orderOf.get(visit.id) ?? 0,
            emoji: visitEmoji(visit),
            name: visitName(visit),
          },
        ]
      : [],
  );
  const hasRoute = track.length > 1 || path.length > 1;
  const empty =
    day.isSuccess && visits.length === 0 && !hasRoute && (summary?.distanceMeters ?? 0) < 1;
  const routeMissing =
    !hasRoute && errorReason(route.error) === "HISTORY_UNAVAILABLE" && visits.length > 0;
  const loading = day.isPending || route.isPending;

  return (
    <Page
      title={title}
      back="/history"
      wide
      actions={
        <div className="flex items-center gap-1">
          <Link
            to="/history/$date"
            params={{ date: shiftDay(date, -1) }}
            aria-label={t("day.previous")}
            className="grid size-9 place-items-center rounded-full hover:bg-surface-2"
          >
            <ChevronLeft className="size-5" />
          </Link>
          {date < today ? (
            <Link
              to="/history/$date"
              params={{ date: shiftDay(date, 1) }}
              aria-label={t("day.next")}
              className="grid size-9 place-items-center rounded-full hover:bg-surface-2"
            >
              <ChevronRight className="size-5" />
            </Link>
          ) : (
            <span className="grid size-9 place-items-center text-ink-faint opacity-40" aria-hidden>
              <ChevronRight className="size-5" />
            </span>
          )}
          <Menu
            trigger={
              <Button variant="ghost" size="icon-sm" aria-label={t("day.more")}>
                <EllipsisVertical className="size-5" />
              </Button>
            }
            actions={[
              {
                label: t("delete.dayAction"),
                icon: <Trash2 className="size-4" />,
                danger: true,
                disabled: empty,
                onSelect: () => setDeleting(true),
              },
            ]}
          />
        </div>
      }
    >
      {loading ? (
        <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_26rem]">
          <Skeleton className="h-72 lg:h-[calc(100dvh-9rem)]" />
          <div className="flex flex-col gap-3">
            <Skeleton className="h-24" />
            <Skeleton className="h-24" />
          </div>
        </div>
      ) : day.isError ? (
        <EmptyState emoji="🌀" title={t("day.failed")} />
      ) : (
        <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_26rem] lg:items-start">
          {!empty && (
            <div className="lg:sticky lg:top-20">
              <DayMap
                date={date}
                track={track}
                path={path}
                stops={stops}
                focus={focus}
                onStop={(id) =>
                  document
                    .getElementById(`visit-${id}`)
                    ?.scrollIntoView({ block: "center", behavior: "smooth" })
                }
                className="sticker h-80 rounded-card lg:h-[calc(100dvh-9rem)]"
              />
              {routeMissing && (
                <p className="mt-2 px-1 text-sm font-bold text-ink-soft">{t("day.routeMissing")}</p>
              )}
            </div>
          )}

          <div
            className={cn(
              "flex flex-col gap-5",
              empty && "lg:col-span-2 lg:mx-auto lg:w-full lg:max-w-2xl",
            )}
          >
            {empty ? (
              <EmptyState
                emoji="🛋️"
                title={t("day.emptyTitle")}
                body={date === today ? t("day.emptyToday") : t("day.emptyBody")}
              />
            ) : (
              <>
                <ul className="flex flex-wrap gap-2" aria-label={t("day.summary")}>
                  <Fact emoji="👟" text={formatDistance(summary?.distanceMeters ?? 0)} />
                  <Fact
                    emoji="📍"
                    text={t("day.visits", { count: summary?.visitCount ?? visits.length })}
                  />
                  {(summary?.newCellCount ?? 0) > 0 && (
                    <Fact
                      emoji="✨"
                      text={t("day.newCells", { count: summary!.newCellCount })}
                      tint="bg-lime text-[#17122b]"
                    />
                  )}
                  {(day.data?.maxSpeedMps ?? 0) > 0.5 && (
                    <Fact
                      emoji="💨"
                      text={t("day.maxSpeed", { speed: kmh(day.data!.maxSpeedMps) })}
                    />
                  )}
                  {summary?.topLocality && <Fact emoji="🗾" text={summary.topLocality} />}
                </ul>

                {(day.data?.friendsMet.length ?? 0) > 0 && (
                  <section
                    className="sticker rounded-card bg-lime p-4 text-[#17122b]"
                    aria-label={t("day.friendsMet")}
                  >
                    <h2 className="font-black">
                      {t("day.friendsMetCount", { count: day.data!.friendsMet.length })}
                    </h2>
                    <ul className="mt-2 flex flex-wrap gap-3">
                      {day.data!.friendsMet.map((friend) => (
                        <li key={friend.id}>
                          <Link
                            to="/u/$handle"
                            params={{ handle: friend.handle }}
                            className="flex w-14 flex-col items-center gap-1"
                          >
                            <Avatar user={friend} size="md" outlined />
                            <span className="w-full truncate text-center text-[11px] font-black">
                              <EmojiText text={friend.displayName} />
                            </span>
                          </Link>
                        </li>
                      ))}
                    </ul>
                  </section>
                )}

                <section aria-label={t("day.timeline")}>
                  <h2 className="text-lg font-black">{t("day.timeline")}</h2>
                  {visits.length === 0 ? (
                    <p className="mt-2 text-sm text-ink-soft">{t("day.noVisits")}</p>
                  ) : (
                    <ol className="mt-3 flex flex-col" data-testid="day-timeline">
                      {timeline.map((item) => {
                        if (item.kind === "move") {
                          const meta = activityMeta(item.activity);
                          return (
                            <li
                              key={`move-${item.from}`}
                              className="ml-[1.3rem] flex items-center gap-3 border-l-[3px] border-dashed border-line py-3 pl-6 text-sm font-bold text-ink-soft"
                              data-kind="move"
                            >
                              <span aria-hidden>{meta.emoji}</span>
                              <span>
                                {t(`activity.${meta.key}`)}・{formatDistance(item.distanceMeters)}・
                                {compactDuration(item.to - item.from)}
                              </span>
                            </li>
                          );
                        }
                        const visit = item.visit;
                        return (
                          <li
                            key={visit.id}
                            id={`visit-${visit.id}`}
                            className="sticker rounded-card bg-surface p-4"
                            data-kind="visit"
                          >
                            <VisitCard
                              visit={visit}
                              onSelect={
                                visit.center
                                  ? () => setFocus({ id: visit.id, nonce: Date.now() })
                                  : undefined
                              }
                              leading={
                                <span
                                  className="sticker-sm relative grid size-11 shrink-0 place-items-center rounded-2xl bg-surface text-xl"
                                  aria-hidden
                                >
                                  {visitEmoji(visit)}
                                  <span className="absolute -top-2 -right-2 grid size-5 place-items-center rounded-full border-2 border-ink bg-lime text-[10px] font-black text-[#17122b]">
                                    {orderOf.get(visit.id)}
                                  </span>
                                </span>
                              }
                            />
                          </li>
                        );
                      })}
                    </ol>
                  )}
                </section>
              </>
            )}

            <DayMemoEditor
              key={`${date}:${day.data?.memo?.updatedAt?.seconds ?? 0}`}
              date={date}
              memo={day.data?.memo}
            />
            {!empty && <GazetteerCredit className="text-center" />}
          </div>
        </div>
      )}

      <DeleteHistoryDialog
        open={deleting}
        onOpenChange={setDeleting}
        date={date}
        onDeleted={() => void navigate({ to: "/history/$date", params: { date }, replace: true })}
      />
    </Page>
  );
}

function Fact({
  emoji,
  text,
  tint = "bg-surface",
}: {
  emoji: string;
  text: string;
  tint?: string;
}) {
  return (
    <li
      className={cn(
        "sticker-sm inline-flex items-center gap-1.5 rounded-full px-3 py-1.5 text-sm font-black",
        tint,
      )}
    >
      <span aria-hidden>{emoji}</span>
      {text}
    </li>
  );
}
