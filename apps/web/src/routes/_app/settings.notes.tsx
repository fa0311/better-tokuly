import { createFileRoute } from "@tanstack/react-router";
import { MutesScreen } from "#/features/notes/mutes-screen";

export const Route = createFileRoute("/_app/settings/notes")({ component: MutesScreen });
