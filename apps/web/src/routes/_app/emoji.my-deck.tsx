import { createFileRoute } from "@tanstack/react-router";
import { DeckScreen } from "#/features/emoji/deck-screen";

export const Route = createFileRoute("/_app/emoji/my-deck")({ component: DeckScreen });
