import { createFileRoute } from "@tanstack/react-router";
import { ShowcaseScreen } from "#/features/quest/showcase-screen";

export const Route = createFileRoute("/_app/quest/showcase")({ component: ShowcaseScreen });
