import { createFileRoute } from "@tanstack/react-router";
import { SourcesScreen } from "#/features/news/sources-screen";

export const Route = createFileRoute("/_app/news/sources")({ component: SourcesScreen });
