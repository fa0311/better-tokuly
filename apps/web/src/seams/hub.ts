import type { LinkProps } from "@tanstack/react-router";
import type { ComponentType } from "react";

/**
 * Hub tiles: every feature's way in from the hub (`/me`), the launcher and
 * the quick search.
 *
 * A feature default-exports its tiles from `src/features/<area>/hub.tsx`:
 *
 *   export default [
 *     {
 *       id: "plans",
 *       section: "play",
 *       order: 20,
 *       to: "/plans",
 *       label: () => i18n.t("plans:tile"),
 *       icon: CalendarHeart,
 *       tint: "bg-lime",
 *       badge: "plans",                 // a key of `#/stores/badges`, optional
 *     },
 *   ] satisfies HubTile[];
 *
 * Nothing else needs editing: the hub, the launcher and the quick search
 * collect the files.
 */
export interface HubTile {
  /** Unique across the app; the area's name, or `<area>-<what>` for a second tile. */
  id: string;
  /** The named group of the hub the tile belongs to. */
  section: HubSectionId;
  /** Position inside the section, ascending. Leave gaps (10, 20, …). */
  order: number;
  to: NonNullable<LinkProps["to"]>;
  /** The tile's name in the current language. */
  label(): string;
  icon: ComponentType<{ className?: string }>;
  /** Background of the icon bubble: one of the ink-on-colour tokens (`bg-sky`, `bg-lime`, `bg-sun`, …). */
  tint: string;
  /** Key of the unread counter to show (`#/stores/badges`). */
  badge?: string;
  /** Only accounts with a staff role see the tile. */
  staffOnly?: boolean;
  /** Other words people look for it by in the quick search ("friends", "ともだち"). */
  keywords?: string[];
  /**
   * A card the hub page shows across the section's full width instead of the
   * plain tile (a widget: the class that is on, a balance). The launcher and
   * the quick search still use the icon and label.
   */
  Card?: ComponentType;
}

/** The hub's sections, in display order. Titles are `hub.sections.<id>` in the `common` strings. */
export const HUB_SECTIONS = ["play", "out", "make", "me"] as const;
export type HubSectionId = (typeof HUB_SECTIONS)[number];

const modules = import.meta.glob<{ default: HubTile[] }>("../features/*/hub.tsx", { eager: true });

/** Every tile, by section and order. */
export const HUB_TILES: HubTile[] = Object.values(modules)
  .flatMap((module) => module.default)
  .sort(
    (a, b) =>
      HUB_SECTIONS.indexOf(a.section) - HUB_SECTIONS.indexOf(b.section) ||
      a.order - b.order ||
      a.id.localeCompare(b.id),
  );

/** The tiles an account sees, grouped by section (empty sections left out). */
export function hubSections(isStaff: boolean): { id: HubSectionId; tiles: HubTile[] }[] {
  return HUB_SECTIONS.map((id) => ({
    id,
    tiles: HUB_TILES.filter((tile) => tile.section === id && (isStaff || !tile.staffOnly)),
  })).filter((section) => section.tiles.length > 0);
}

/** The badge keys of every tile: what the launcher's own entry sums up. */
export const HUB_BADGES: string[] = [
  ...new Set(HUB_TILES.flatMap((tile) => (tile.badge ? [tile.badge] : []))),
];
