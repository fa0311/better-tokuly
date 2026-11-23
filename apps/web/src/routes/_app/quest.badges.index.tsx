import { createFileRoute } from "@tanstack/react-router";
import { BadgesScreen } from "#/features/quest/badges-screen";

export const Route = createFileRoute("/_app/quest/badges/")({ component: BadgesScreen });
