import { Link } from "@tanstack/react-router";
import { Suspense } from "react";
import { useTranslation } from "react-i18next";
import { CountBadge } from "#/components/ui/badge";
import { useStaffAccess } from "#/features/admin";
import { cn } from "#/lib/cn";
import { HUB_NAMESPACES, type HubTile, hubSections } from "#/seams/hub";
import { useBadge } from "#/stores/badges";

/**
 * Every feature's way in, grouped into the hub's sections. The tiles come
 * from the features themselves (`src/features/<area>/hub.tsx`, see
 * `#/seams/hub`).
 *
 * - `page`: the hub page — roomy rows, and a feature's own card where it has one;
 * - `launcher`: an app drawer of icons, for a sheet over whatever page is open.
 */
export function HubSections(props: HubSectionsProps) {
  // The page shows its cards with the tiles, so nothing moves when a card arrives.
  if (props.variant === "page") {
    return (
      <Suspense fallback={null}>
        <Sections {...props} />
      </Suspense>
    );
  }
  return <Sections {...props} />;
}

interface HubSectionsProps {
  variant: "page" | "launcher";
  /** Called when a tile is followed (the launcher closes itself). */
  onNavigate?(): void;
}

function Sections({ variant, onNavigate }: HubSectionsProps) {
  // The section titles follow the language; the tiles' own labels are read while rendering.
  const { t } = useTranslation("common");
  const isStaff = useStaffAccess().staff;
  return (
    <div className={cn("flex flex-col", variant === "page" ? "gap-7" : "gap-5")}>
      {hubSections(isStaff).map((section) => (
        <section key={section.id} aria-labelledby={`hub-${variant}-${section.id}`}>
          <h2
            id={`hub-${variant}-${section.id}`}
            className={cn(
              "px-1 font-black",
              variant === "page" ? "text-lg" : "text-sm text-ink-soft",
            )}
          >
            {t(`hub.sections.${section.id}`)}
          </h2>
          <ul
            className={cn(
              "mt-2.5 grid",
              variant === "page"
                ? "grid-cols-2 gap-2.5 sm:grid-cols-3"
                : "grid-cols-4 gap-x-1 gap-y-3 sm:grid-cols-5",
            )}
          >
            {section.tiles.map((tile) =>
              variant === "page" && tile.Card ? (
                <li key={tile.id} className="col-span-full">
                  <tile.Card />
                </li>
              ) : (
                <li key={tile.id}>
                  {variant === "page" ? (
                    <PageTile tile={tile} />
                  ) : (
                    <LauncherTile tile={tile} onNavigate={onNavigate} />
                  )}
                </li>
              ),
            )}
          </ul>
        </section>
      ))}
    </div>
  );
}

function IconBubble({ tile, className }: { tile: HubTile; className?: string }) {
  const Icon = tile.icon;
  return (
    <span
      className={cn(
        "grid shrink-0 place-items-center rounded-2xl border-2 border-ink text-[#17122b]",
        tile.tint,
        className,
      )}
    >
      <Icon className="size-[22px]" />
    </span>
  );
}

/** The tile's name; empty for the moment its namespace takes to load. */
function useTileName(tile: HubTile): string {
  const { t } = useTranslation(HUB_NAMESPACES, { useSuspense: false });
  return t(tile.label);
}

function PageTile({ tile }: { tile: HubTile }) {
  const count = useBadge(tile.badge);
  const name = useTileName(tile);
  return (
    <Link
      to={tile.to}
      className="pressable sticker flex h-[4.25rem] items-center gap-2.5 rounded-card bg-surface px-3"
    >
      <span className="relative">
        <IconBubble tile={tile} className="size-11" />
        <CountBadge count={count} className="absolute -top-1.5 -right-1.5" />
      </span>
      <span className="line-clamp-2 min-w-0 flex-1 text-[15px] leading-tight font-black">
        {name}
      </span>
    </Link>
  );
}

function LauncherTile({ tile, onNavigate }: { tile: HubTile; onNavigate?(): void }) {
  const count = useBadge(tile.badge);
  const name = useTileName(tile);
  return (
    <Link
      to={tile.to}
      onClick={onNavigate}
      className="group flex flex-col items-center gap-1.5 rounded-2xl px-1 py-1.5 text-center transition-colors hover:bg-surface-2"
    >
      <span className="relative">
        <IconBubble tile={tile} className="size-12 transition-transform group-active:scale-95" />
        <CountBadge count={count} className="absolute -top-1.5 -right-1.5" />
      </span>
      <span className="line-clamp-2 w-full text-[11px] leading-tight font-bold [overflow-wrap:anywhere]">
        {name}
      </span>
    </Link>
  );
}
