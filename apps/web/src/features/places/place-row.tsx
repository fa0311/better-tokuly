import { Link } from "@tanstack/react-router";
import { type geo, toMillis } from "@tokuly/api";
import { labelMeta, placeEmoji } from "@tokuly/core/places";
import { ChevronRight } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import { compactAge } from "#/lib/format";

/** The emoji of a place in its coloured sticker bubble. */
export function PlaceBubble({
  place,
  size = "md",
}: {
  place: Pick<geo.Place, "emoji" | "label">;
  size?: "md" | "lg";
}) {
  return (
    <span
      className={cn(
        "grid shrink-0 place-items-center border-2 border-ink",
        labelMeta(place.label).tint,
        size === "lg" ? "size-16 rounded-3xl text-3xl" : "size-12 rounded-2xl text-2xl",
      )}
      aria-hidden
    >
      {placeEmoji(place)}
    </span>
  );
}

/** One place in a list, leading to its page. */
export function PlaceRow({
  place,
  subtitle,
  trailing,
}: {
  place: geo.Place;
  subtitle?: ReactNode;
  trailing?: ReactNode;
}) {
  const { t } = useTranslation("places");
  const lastVisit = toMillis(place.myLastVisitAt);
  const facts = [
    place.myVisitCount > 0 ? t("row.visits", { count: place.myVisitCount }) : "",
    lastVisit > 0 ? t("row.lastVisit", { age: compactAge(lastVisit) }) : "",
  ].filter(Boolean);
  return (
    <Link
      to="/places/$placeId"
      params={{ placeId: place.id }}
      className="pressable sticker flex items-center gap-3 rounded-card bg-surface p-3"
    >
      <PlaceBubble place={place} />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-black">{place.name}</span>
        <span className="block truncate text-sm text-ink-soft">
          {subtitle ?? (facts.join(" · ") || place.address || t("row.noVisits"))}
        </span>
      </span>
      {trailing ?? <ChevronRight className="size-5 shrink-0 text-ink-faint" />}
    </Link>
  );
}
