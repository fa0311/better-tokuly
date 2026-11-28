import type { ComponentType } from "react";
import { onDemand } from "#/components/on-demand";

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

// One that fails to load says so in its place, and loads again (`onDemand`).
const cards = Object.entries(modules).map(
  ([path, load]) =>
    [
      path.split("/").at(-2) ?? path,
      onDemand(() => load().then((module) => module.default), "notice"),
    ] as const,
);

/**
 * Settles once every card has arrived or failed to. The hub page's route
 * waits for it, so the page shows whole: rendered before, the sections would
 * come in under the heading a moment later, pushing what is below them down.
 */
export const hubCardsLoaded: Promise<unknown> = Promise.all(
  cards.map(([, Card]) => Card.preload()),
);

/** Every area's card, by the id of the tile it replaces. */
export const HUB_CARDS: Record<string, ComponentType> = Object.fromEntries(cards);
