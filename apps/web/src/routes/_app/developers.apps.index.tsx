import { createFileRoute } from "@tanstack/react-router";
import { AppsScreen } from "#/features/dev/apps";

export const Route = createFileRoute("/_app/developers/apps/")({ component: AppsScreen });
