import type { LinkProps } from "@tanstack/react-router";
import { type ComponentType, createElement, lazy } from "react";
import { namespacesOf } from "#/lib/i18n";

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
 *       label: "plans:tile",
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
  /** The key of the tile's name, with its namespace: `"plans:tile"`. */
  label: string;
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
   * plain tile (a widget: the class that is on, a balance), as a dynamic
   * import: `card: () => import("./hub-card")`, so it is not in the app's
   * entry; keep it small, it is fetched as soon as the tiles are. The launcher and the quick search use the icon and label.
   */
  card?: () => Promise<{ default: ComponentType }>;
}

/** A tile as the hub renders it: its card, if any, ready to suspend while it loads. */
export type LoadedHubTile = Omit<HubTile, "card"> & { Card?: ComponentType };

/** The hub's sections, in display order. Titles are `hub.sections.<id>` in the `common` strings. */
export const HUB_SECTIONS = ["play", "out", "make", "me"] as const;
export type HubSectionId = (typeof HUB_SECTIONS)[number];

const modules = import.meta.glob<{ default: HubTile[] }>("../features/*/hub.tsx", { eager: true });

/**
 * A card fetched with the tiles (they are small), so the hub rarely waits for
 * one. Once it is there it renders at once: `lazy` alone would still suspend
 * on its first render, and React then holds the hub back for up to 300 ms
 * after the page's heading has shown (the tiles moved in under it).
 */
function preloaded(load: NonNullable<HubTile["card"]>): ComponentType {
  let Loaded: ComponentType | undefined;
  const loading = load().then((module) => {
    Loaded = module.default;
    return module;
  });
  const Waiting = lazy(() => loading);
  return function Card() {
    return createElement(Loaded ?? Waiting);
  };
}

/** Every tile, by section and order. */
export const HUB_TILES: LoadedHubTile[] = Object.values(modules)
  .flatMap((module) => module.default)
  .map(({ card, ...tile }) => (card ? { ...tile, Card: preloaded(card) } : tile))
  .sort(
    (a, b) =>
      HUB_SECTIONS.indexOf(a.section) - HUB_SECTIONS.indexOf(b.section) ||
      a.order - b.order ||
      a.id.localeCompare(b.id),
  );

/** What `useTranslation` loads before the tiles' names show. */
export const HUB_NAMESPACES = namespacesOf(HUB_TILES.map((tile) => tile.label));

/** The tiles an account sees, grouped by section (empty sections left out). */
export function hubSections(isStaff: boolean): { id: HubSectionId; tiles: LoadedHubTile[] }[] {
  return HUB_SECTIONS.map((id) => ({
    id,
    tiles: HUB_TILES.filter((tile) => tile.section === id && (isStaff || !tile.staffOnly)),
  })).filter((section) => section.tiles.length > 0);
}
