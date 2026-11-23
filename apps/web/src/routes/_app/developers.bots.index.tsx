import { createFileRoute } from "@tanstack/react-router";
import { BotsScreen } from "#/features/dev/bots";

export const Route = createFileRoute("/_app/developers/bots/")({ component: BotsScreen });
