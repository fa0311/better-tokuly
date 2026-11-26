import { Link } from "@tanstack/react-router";
import { stamp } from "@tokuly/api";
import { statusKey } from "@tokuly/core/stamps";
import { BadgeCheck, Check, Clapperboard, Heart, Search, Sparkles, X } from "lucide-react";
import { type ReactNode, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Chip } from "#/components/ui/card";
import { Coins } from "#/components/ui/coins";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { StampImage } from "./stamp-image";

/** A pack's cover on a soft tile tinted with the cover's own colour. */
export function PackCover({
  pack,
  className,
}: {
  pack: Pick<stamp.PackSummary, "cover" | "name">;
  className?: string;
}) {
  const cover = pack.cover;
  // The middle rendition is plenty for cards; the full picture only when there is nothing smaller.
  const url = cover
    ? (cover.variants.filter((variant) => variant.size >= 160).sort((a, b) => a.size - b.size)[0]
        ?.url ?? cover.url)
    : undefined;
  return (
    <span
      className={cn(
        "relative block aspect-square overflow-hidden rounded-2xl bg-surface-2",
        className,
      )}
      style={
        cover?.dominantColor
          ? { background: `color-mix(in srgb, ${cover.dominantColor} 24%, var(--surface-2))` }
          : undefined
      }
    >
      {/* Placed by offsets, so the picture has its size whether the tile gets its own from a width, a height or its proportions. */}
      {url ? (
        <img
          src={url}
          alt=""
          className="absolute inset-[12%] h-[76%] w-[76%] object-contain drop-shadow-[0_2px_0_rgb(23_18_43/0.18)]"
          loading="lazy"
          decoding="async"
          draggable={false}
        />
      ) : (
        <Sparkles className="absolute inset-1/4 size-1/2 text-ink-faint" aria-hidden />
      )}
    </span>
  );
}

/** Who made a pack: the team's badge, or the creator's name. */
export function PackMaker({
  pack,
  className,
}: {
  pack: Pick<stamp.PackSummary, "official" | "creator">;
  className?: string;
}) {
  const { t } = useTranslation("stamps");
  if (pack.official) {
    return (
      <span className={cn("inline-flex items-center gap-1 font-bold text-brand", className)}>
        <BadgeCheck className="size-3.5 shrink-0" />
        <span className="truncate">{t("official")}</span>
      </span>
    );
  }
  return (
    <span className={cn("truncate text-ink-soft", className)}>
      {pack.creator?.deleted ? t("deletedCreator") : (pack.creator?.displayName ?? "")}
    </span>
  );
}

/** Price of a pack, or what the viewer already has of it. */
export function PriceTag({
  pack,
  className,
}: {
  pack: Pick<stamp.PackSummary, "price" | "viewer">;
  className?: string;
}) {
  const { t } = useTranslation("stamps");
  if (pack.viewer?.installed) {
    return (
      <span
        className={cn(
          "inline-flex items-center gap-0.5 text-xs font-black text-success",
          className,
        )}
      >
        <Check className="size-3.5" strokeWidth={3} />
        {t("state.installed")}
      </span>
    );
  }
  if (pack.viewer?.owned)
    return (
      <span className={cn("text-xs font-black text-brand", className)}>{t("state.owned")}</span>
    );
  if (pack.price === 0)
    return <span className={cn("text-xs font-black text-ink-soft", className)}>{t("free")}</span>;
  return <Coins amount={pack.price} size="sm" className={className} />;
}

/** The little "it moves" mark on packs with animated stamps. */
export function AnimatedMark({ className }: { className?: string }) {
  const { t } = useTranslation("stamps");
  return (
    <span
      title={t("animated")}
      className={cn(
        "inline-grid size-5 place-items-center rounded-full border-2 border-ink bg-sun text-[#17122b]",
        className,
      )}
    >
      <Clapperboard className="size-3" aria-label={t("animated")} />
    </span>
  );
}

/** A pack as a card on a shelf or in a grid. */
export function PackCard({ pack, className }: { pack: stamp.PackSummary; className?: string }) {
  return (
    <Link
      to="/stamps/$packId"
      params={{ packId: pack.shortId || pack.id }}
      className={cn(
        "pressable sticker flex min-w-0 flex-col gap-2 rounded-card bg-surface p-2.5",
        className,
      )}
      data-testid="pack-card"
    >
      <span className="relative block">
        <PackCover pack={pack} />
        {pack.animated && <AnimatedMark className="absolute top-1.5 right-1.5" />}
      </span>
      <span className="flex min-w-0 flex-col px-1 pb-1">
        <span className="line-clamp-2 font-black leading-tight break-words">{pack.name}</span>
        <PackMaker pack={pack} className="mt-0.5 text-xs" />
        <PriceTag pack={pack} className="mt-1.5" />
      </span>
    </Link>
  );
}

/** A pack as a row: ranked lists, the library, search results. */
export function PackRow({
  pack,
  rank,
  trailing,
  className,
}: {
  pack: stamp.PackSummary;
  rank?: number;
  trailing?: ReactNode;
  className?: string;
}) {
  const { t } = useTranslation("stamps");
  return (
    <div className={cn("flex items-center gap-3 rounded-2xl px-2 py-2", className)}>
      {rank !== undefined && (
        <span
          className={cn(
            "w-6 shrink-0 text-center text-lg font-black tabular-nums",
            rank <= 3 ? "text-pink" : "text-ink-faint",
          )}
        >
          {rank}
        </span>
      )}
      <Link
        to="/stamps/$packId"
        params={{ packId: pack.shortId || pack.id }}
        className="flex min-w-0 flex-1 items-center gap-3"
      >
        <PackCover pack={pack} className="size-14 shrink-0 rounded-xl" />
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-1.5">
            <span className="truncate font-black">{pack.name}</span>
            {pack.animated && <AnimatedMark className="shrink-0" />}
          </span>
          <span className="flex min-w-0 items-center gap-1.5 text-xs">
            <PackMaker pack={pack} />
            <span className="shrink-0 text-ink-faint">
              · {t("stampCount", { count: pack.stampCount })}
            </span>
          </span>
        </span>
      </Link>
      {trailing ?? <PriceTag pack={pack} className="shrink-0" />}
    </div>
  );
}

const STATUS_TINT: Record<number, string> = {
  [stamp.PackStatus.DRAFT]: "bg-surface-3 text-ink-soft",
  [stamp.PackStatus.IN_REVIEW]: "bg-sun text-[#17122b]",
  [stamp.PackStatus.PUBLISHED]: "bg-lime text-[#17122b]",
  [stamp.PackStatus.REJECTED]: "bg-danger text-white",
  [stamp.PackStatus.REMOVED]: "bg-danger text-white",
};

/** Where a pack is in its lifecycle, as a coloured label (the text carries the meaning). */
export function StatusChip({
  status,
  className,
}: {
  status: stamp.PackStatus;
  className?: string;
}) {
  const { t } = useTranslation("stamps");
  return (
    <Chip className={cn(STATUS_TINT[status] ?? STATUS_TINT[stamp.PackStatus.DRAFT], className)}>
      {t(statusKey(status))}
    </Chip>
  );
}

/** Heading of a shelf, with an optional "more" link on the right. */
export function SectionHeading({
  title,
  action,
  className,
}: {
  title: ReactNode;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex items-end justify-between gap-3 px-1 pb-2", className)}>
      <h2 className="text-lg font-black">{title}</h2>
      {action}
    </div>
  );
}

/** A search box with a clear button. */
export function SearchField({
  value,
  onChange,
  onSubmit,
  placeholder,
  autoFocus,
  className,
}: {
  value: string;
  onChange(value: string): void;
  onSubmit?(): void;
  placeholder: string;
  autoFocus?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("stamps");
  const input = useRef<HTMLInputElement>(null);
  // Arriving with a search already typed, the cursor is in the box to keep typing.
  useEffect(() => {
    if (autoFocus) input.current?.focus();
  }, [autoFocus]);
  return (
    <form
      className={cn(
        "flex h-12 items-center gap-2 rounded-control border-2 border-line bg-surface px-4 focus-within:border-brand",
        className,
      )}
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit?.();
      }}
    >
      <Search className="size-5 shrink-0 text-ink-faint" />
      <input
        type="search"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        aria-label={placeholder}
        ref={input}
        maxLength={64}
        enterKeyHint="search"
        className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-ink-faint [&::-webkit-search-cancel-button]:hidden"
      />
      {value && (
        <button
          type="button"
          aria-label={t("search.clear")}
          onClick={() => onChange("")}
          className="grid size-7 shrink-0 place-items-center rounded-full bg-surface-2 text-ink-soft hover:bg-surface-3"
        >
          <X className="size-4" />
        </button>
      )}
    </form>
  );
}

/** A stamp in a grid. The whole tile is the button; marks sit in its corners. */
export function StampTile({
  item,
  onPress,
  play,
  hidePending,
  className,
  children,
}: {
  item: stamp.Stamp;
  onPress?(item: stamp.Stamp): void;
  /** Leave out the "awaiting review" mark (where every stamp would carry it). */
  hidePending?: boolean;
  play?: "visible" | "always" | "never";
  className?: string;
  /** Extra marks drawn over the tile. */
  children?: ReactNode;
}) {
  const { t } = useTranslation("stamps");
  const ref = item.stampRef;
  if (!ref) return null;
  return (
    <div className={cn("relative", className)} data-testid="stamp-tile">
      <button
        type="button"
        onClick={() => onPress?.(item)}
        aria-label={ref.name || t("stamp.unnamed")}
        className="block w-full rounded-2xl p-1.5 transition-transform duration-150 hover:bg-surface-2 active:scale-90"
      >
        <StampImage stamp={ref} image={item.image} play={play} className="block" />
      </button>
      {item.favorited && (
        <span className="pointer-events-none absolute top-0.5 right-0.5 grid size-5 place-items-center rounded-full border-2 border-ink bg-pink text-white">
          <Heart className="size-2.5 fill-current" aria-label={t("stamp.favorited")} />
        </span>
      )}
      {item.pendingReview && !hidePending && (
        <span className="pointer-events-none absolute bottom-0.5 left-1/2 -translate-x-1/2 rounded-full border-2 border-ink bg-sun px-1.5 text-[10px] font-black whitespace-nowrap text-[#17122b]">
          {t("stamp.pending")}
        </span>
      )}
      {children}
    </div>
  );
}

/** Grid for stamp tiles: four across on phones, more as the screen widens. */
export function StampGrid({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "grid grid-cols-4 gap-1 sm:grid-cols-5 md:grid-cols-6 lg:grid-cols-8",
        className,
      )}
    >
      {children}
    </div>
  );
}

/** Placeholder for a grid of packs that is still loading. */
export function PackGridSkeleton({ count = 6, className }: { count?: number; className?: string }) {
  return (
    <div
      className={cn(
        "grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6",
        className,
      )}
    >
      {Array.from({ length: count }, (_, index) => (
        <Skeleton key={index} className="aspect-[4/5]" />
      ))}
    </div>
  );
}

/** A tag people can follow into the store. */
export function TagLink({ tag, count }: { tag: string; count?: number }) {
  return (
    <Link
      to="/stamps/browse"
      search={{ tag }}
      className="pressable sticker-sm inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full bg-surface px-3.5 text-sm font-bold"
    >
      <span className="text-brand">#</span>
      {tag}
      {count !== undefined && <span className="text-xs text-ink-faint">{count}</span>}
    </Link>
  );
}
