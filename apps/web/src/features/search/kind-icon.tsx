import { search } from "@tokuly/api";
import {
  Clapperboard,
  Hash,
  ListVideo,
  MapPin,
  Newspaper,
  Radio,
  Scissors,
  Smartphone,
  Smile,
  StickyNote,
  Tv,
  UserRound,
} from "lucide-react";
import type { ComponentType } from "react";
import { cn } from "#/lib/cn";

const K = search.SearchKind;

/** What each kind looks like wherever it has no picture of its own: an icon on its sticker colour. */
const KIND_STYLE: Record<number, { icon: ComponentType<{ className?: string }>; tint: string }> = {
  [K.USER]: { icon: UserRound, tint: "bg-sky text-[#17122b]" },
  [K.CHANNEL]: { icon: Tv, tint: "bg-coral text-[#17122b]" },
  [K.NOTE]: { icon: StickyNote, tint: "bg-sun text-[#17122b]" },
  [K.VIDEO]: { icon: Clapperboard, tint: "bg-pink text-white" },
  [K.SHORT]: { icon: Smartphone, tint: "bg-pink text-white" },
  [K.LIVE]: { icon: Radio, tint: "bg-live text-white" },
  [K.CLIP]: { icon: Scissors, tint: "bg-mint text-[#17122b]" },
  [K.PLAYLIST]: { icon: ListVideo, tint: "bg-brand text-on-brand" },
  [K.STAMP_PACK]: { icon: Smile, tint: "bg-sun text-[#17122b]" },
  [K.PLACE]: { icon: MapPin, tint: "bg-mint text-[#17122b]" },
  [K.HASHTAG]: { icon: Hash, tint: "bg-lime text-[#17122b]" },
  [K.NEWS]: { icon: Newspaper, tint: "bg-sky text-[#17122b]" },
};

const FALLBACK = KIND_STYLE[K.NOTE] as (typeof KIND_STYLE)[number];

export function kindIcon(kind: search.SearchKind): ComponentType<{ className?: string }> {
  return (KIND_STYLE[kind] ?? FALLBACK).icon;
}

/** A kind's icon in a small rounded tile (suggestion rows, results without an image). */
export function KindTile({
  kind,
  className,
  iconClassName,
}: {
  kind: search.SearchKind;
  className?: string;
  iconClassName?: string;
}) {
  const { icon: Icon, tint } = KIND_STYLE[kind] ?? FALLBACK;
  return (
    <span
      aria-hidden
      className={cn("grid size-11 shrink-0 place-items-center rounded-[0.85rem]", tint, className)}
    >
      <Icon className={cn("size-5", iconClassName)} />
    </span>
  );
}
