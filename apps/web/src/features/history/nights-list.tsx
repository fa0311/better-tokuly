import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { toDate } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { GazetteerCredit } from "#/features/places";
import { cn } from "#/lib/cn";
import { historyClient, historyKeys } from "./api";
import { parseDay } from "./dates";

/** Where I slept, night by night; the switch keeps only the nights away from home. */
export function NightsList() {
  const { t, i18n } = useTranslation("history");
  const [awayOnly, setAwayOnly] = useState(false);
  const query = useInfiniteQuery({
    queryKey: historyKeys.nights,
    queryFn: ({ pageParam }) =>
      historyClient().listNights({ page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const all = query.data?.pages.flatMap((page) => page.nights) ?? [];
  const nights = awayOnly ? all.filter((night) => !night.atHome) : all;
  const clock = (date: Date | undefined) =>
    date?.toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" }) ?? "";

  return (
    <div className="mx-auto max-w-2xl">
      <div className="rounded-card bg-surface p-2">
        <SwitchRow
          label={t("nights.awayOnly")}
          hint={t("nights.awayOnlyHint")}
          checked={awayOnly}
          onChange={setAwayOnly}
        />
      </div>
      <div className="mt-4">
        {query.isPending ? (
          <Skeleton className="h-20" />
        ) : nights.length === 0 ? (
          all.length === 0 ? (
            <EmptyState emoji="🌙" title={t("nights.emptyTitle")} body={t("nights.emptyBody")} />
          ) : (
            <EmptyState emoji="🏠" title={t("nights.emptyAway")} />
          )
        ) : (
          <>
            <ul className="flex flex-col gap-3">
              {nights.map((night) => {
                const date = parseDay(night.date);
                const name = night.atHome
                  ? t("nights.home")
                  : night.place?.name || night.name || t("nights.somewhere");
                return (
                  <li key={night.date}>
                    <Link
                      to="/history/$date"
                      params={{ date: night.date }}
                      className="pressable sticker flex items-center gap-3 rounded-card bg-surface p-3"
                    >
                      <span
                        className={cn(
                          "grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink text-2xl",
                          night.atHome ? "bg-sun" : "bg-brand",
                        )}
                        aria-hidden
                      >
                        {night.atHome ? "🏠" : "🌙"}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-black">{name}</span>
                        <span className="block truncate text-sm text-ink-soft tabular-nums">
                          {date?.toLocaleDateString(i18n.language, {
                            month: "short",
                            day: "numeric",
                            weekday: "short",
                          })}{" "}
                          {clock(toDate(night.from))}–{clock(toDate(night.to))}
                        </span>
                      </span>
                      {night.withFriends.length > 0 && (
                        <span
                          className="flex shrink-0 -space-x-2"
                          title={t("visit.with", { count: night.withFriends.length })}
                        >
                          {night.withFriends.slice(0, 3).map((friend) => (
                            <Avatar key={friend.id} user={friend} size="xs" />
                          ))}
                        </span>
                      )}
                    </Link>
                  </li>
                );
              })}
            </ul>
            <LoadMore query={query} />
            <GazetteerCredit className="mt-4 text-center" />
          </>
        )}
      </div>
    </div>
  );
}
