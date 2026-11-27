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

const loads = Object.entries(modules).map(([path, load]) => {
  const loading = load();
  return { id: path.split("/").at(-2) ?? path, loading };
});

/**
 * Settles once every card has arrived. The hub page's route waits for it, so
 * the page shows whole: rendered before, the sections would come in under
 * the heading a moment later, pushing what is below them down.
 */
export const hubCardsLoaded: Promise<unknown> = Promise.allSettled(
  loads.map(({ loading }) => loading),
);

/**
 * A card that renders at once when it has arrived: `lazy` alone would still
 * suspend on its first render, and React then holds the hub back for up to
 * 300 ms. `lazy` only covers a card whose load failed or is still on its way.
 */
function preloaded(loading: Promise<{ default: ComponentType }>): ComponentType {
  let Loaded: ComponentType | undefined;
  void loading.then((module) => {
    Loaded = module.default;
  });
  const Waiting = lazy(() => loading);
  return function Card() {
    return createElement(Loaded ?? Waiting);
  };
}

/** Every area's card, by the id of the tile it replaces. */
export const HUB_CARDS: Record<string, ComponentType> = Object.fromEntries(
  loads.map(({ id, loading }) => [id, preloaded(loading)]),
);
