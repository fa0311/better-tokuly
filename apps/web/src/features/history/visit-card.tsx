import { Link } from "@tanstack/react-router";
import { type geo, toDate } from "@tokuly/api";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { placeEmoji } from "#/features/places";
import { compactDuration } from "#/lib/format";

/** The emoji a visit is drawn with: its place's, a tick for a check-in, footprints otherwise. */
export const visitEmoji = (visit: geo.Visit): string => (visit.place ? placeEmoji(visit.place) : visit.checkIn ? "✅" : "👣");

/** What to call a visit: the place, else the name the server gave the spot. */
export const visitName = (visit: geo.Visit): string => visit.place?.name || visit.name || visit.locality;

const clock = (date: Date | undefined, language: string) => date?.toLocaleTimeString(language, { hour: "2-digit", minute: "2-digit" }) ?? "";

/** "17:16–17:25・9分", "17:33〜 いまここ" or just the time of a check-in. */
export function useVisitTimes(): (visit: geo.Visit) => string {
  const { t, i18n } = useTranslation("history");
  return (visit) => {
    const arrivedAt = toDate(visit.arrivedAt);
    const departed = toDate(visit.departedAt);
    // A stay that began on an earlier day (a night at home) says which day.
    const overnight = arrivedAt && departed && arrivedAt.toDateString() !== departed.toDateString();
    const arrived = `${overnight ? `${arrivedAt.toLocaleDateString(i18n.language, { month: "numeric", day: "numeric" })} ` : ""}${clock(arrivedAt, i18n.language)}`;
    const seconds = Number(visit.durationSeconds);
    if (departed) return seconds >= 60 ? `${arrived}–${clock(departed, i18n.language)}・${compactDuration(seconds * 1000)}` : arrived;
    return visit.checkIn && seconds < 60 ? arrived : t("visit.ongoing", { time: arrived });
  };
}

/** The body of a visit wherever one is listed: emoji, name, when, where, with whom, the check-in message. */
export function VisitCard({ visit, leading, onSelect, dateLabel }: { visit: geo.Visit; leading?: ReactNode; onSelect?(): void; /** A date to show before the time (lists that span days). */ dateLabel?: string }) {
  const { t } = useTranslation("history");
  const times = useVisitTimes();
  const name = visitName(visit);
  return (
    <div className="flex items-start gap-3">
      {leading ?? (
        <span className="sticker-sm grid size-11 shrink-0 place-items-center rounded-2xl bg-surface text-xl" aria-hidden>
          {visitEmoji(visit)}
        </span>
      )}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          {onSelect ? (
            <button type="button" onClick={onSelect} className="min-w-0 truncate text-left font-black hover:underline">
              {name}
            </button>
          ) : (
            <span className="min-w-0 truncate font-black">{name}</span>
          )}
          {visit.checkIn && <span className="shrink-0 rounded-full bg-lime px-2 py-0.5 text-[10px] font-black text-[#17122b]">{t("visit.checkIn")}</span>}
        </div>
        <p className="text-sm font-bold text-ink-soft tabular-nums">
          {dateLabel && `${dateLabel} `}
          {times(visit)}
        </p>
        {visit.locality && visit.locality !== name && <p className="truncate text-xs text-ink-faint">{visit.locality}</p>}
        {visit.message && <p className="mt-1.5 rounded-2xl bg-surface-2 px-3 py-2 text-sm">{visit.message}</p>}
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
          <Link to="/places/$placeId" params={{ placeId: visit.place.id }} className="mt-1 inline-block text-xs font-black text-brand hover:underline">
            {t("visit.openPlace")}
          </Link>
        )}
      </div>
    </div>
  );
}
