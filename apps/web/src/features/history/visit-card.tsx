import { Link } from "@tanstack/react-router";
import type { geo } from "@tokuly/api";
import { visitEmoji, visitName, visitTimes } from "@tokuly/core/history";
import type { ReactNode } from "react";
import { Avatar } from "#/components/ui/avatar";
import { accountTimeZone } from "#/lib/account-time";
import { useTranslation } from "#/lib/use-translation";

/** "17:16–17:25・9分", "17:33〜 いまここ" or just the time of a check-in, on the account's clock. */
export function useVisitTimes(): (visit: geo.Visit) => string {
  const { i18n } = useTranslation("history");
  return (visit) => visitTimes(i18n, visit, accountTimeZone());
}

/** The body of a visit wherever one is listed: emoji, name, when, where, with whom, the check-in message. */
export function VisitCard({
  visit,
  leading,
  onSelect,
  dateLabel,
}: {
  visit: geo.Visit;
  leading?: ReactNode;
  onSelect?(): void /** A date to show before the time (lists that span days). */;
  dateLabel?: string;
}) {
  const { t } = useTranslation("history");
  const times = useVisitTimes();
  const name = visitName(visit);
  return (
    <div className="flex items-start gap-3">
      {leading ?? (
        <span
          className="sticker-sm grid size-11 shrink-0 place-items-center rounded-2xl bg-surface text-xl"
          aria-hidden
        >
          {visitEmoji(visit)}
        </span>
      )}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          {onSelect ? (
            <button
              type="button"
              onClick={onSelect}
              className="min-w-0 truncate text-left font-black hover:underline"
            >
              {name}
            </button>
          ) : (
            <span className="min-w-0 truncate font-black">{name}</span>
          )}
          {visit.checkIn && (
            <span className="shrink-0 rounded-full bg-lime px-2 py-0.5 text-[10px] font-black text-[#17122b]">
              {t("visit.checkIn")}
            </span>
          )}
        </div>
        <p className="text-sm font-bold text-ink-soft tabular-nums">
          {dateLabel && `${dateLabel} `}
          {times(visit)}
        </p>
        {visit.locality && visit.locality !== name && (
          <p className="truncate text-xs text-ink-faint">{visit.locality}</p>
        )}
        {visit.message && (
          <p className="mt-1.5 rounded-2xl bg-surface-2 px-3 py-2 text-sm">{visit.message}</p>
        )}
        {visit.withFriends.length > 0 && (
          <p className="mt-1.5 flex items-center gap-2 text-xs font-bold text-ink-soft">
            <span className="flex -space-x-2">
              {visit.withFriends.slice(0, 5).map((friend) => (
                <Avatar key={friend.id} user={friend} size="xs" />
              ))}
            </span>
            {t("visit.with", { count: visit.withFriends.length })}
          </p>
        )}
        {visit.place && (
          <Link
            to="/places/$placeId"
            params={{ placeId: visit.place.id }}
            className="mt-1 inline-block text-xs font-black text-brand hover:underline"
          >
            {t("visit.openPlace")}
          </Link>
        )}
      </div>
    </div>
  );
}
