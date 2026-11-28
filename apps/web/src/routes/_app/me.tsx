import { createFileRoute } from "@tanstack/react-router";
import { MeScreen } from "#/features/me/me-screen";
import { loadNow } from "#/lib/load-module";

export const Route = createFileRoute("/_app/me")({
  // The hub shows with its cards, never before them (`#/seams/hub-cards`).
  loader: () => loadNow(() => import("#/seams/hub-cards")).then((cards) => cards.hubCardsLoaded),
  component: MeScreen,
});
