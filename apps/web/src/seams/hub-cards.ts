import { type ComponentType, createElement, lazy } from "react";

/**
 * Hub cards: a widget the hub page (`/me`) shows across its section instead
 * of an area's plain tile (the class that is on, today's missions). An area
 * default-exports it from `src/features/<area>/hub-card.tsx`; it replaces the
 * tile whose `id` is the area's name. The launcher and the quick search keep
 * the plain tile.
 *
 * Only the hub page imports this module, so the cards load with it, never
 * on the other pages, and they count towards its size
 * (`scripts/startup-size.mjs`): keep them small.
 */
const modules = import.meta.glob<{ default: ComponentType }>("../features/*/hub-card.tsx");

/**
 * A card fetched as soon as the hub page's code is there, next to the page's
 * own requests. Once it has arrived it renders at once: `lazy` alone would
 * still suspend on its first render, and React then holds the hub back for
 * up to 300 ms after the page's heading has shown (the tiles moved in under it).
 */
function preloaded(load: () => Promise<{ default: ComponentType }>): ComponentType {
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

/** Every area's card, by the id of the tile it replaces. */
export const HUB_CARDS: Record<string, ComponentType> = Object.fromEntries(
  Object.entries(modules).map(([path, load]) => [path.split("/").at(-2) ?? path, preloaded(load)]),
);
