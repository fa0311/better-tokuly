import { createFileRoute } from "@tanstack/react-router";
import { DevelopersScreen } from "#/features/dev/developers-screen";

export const Route = createFileRoute("/_app/developers/")({ component: DevelopersScreen });
