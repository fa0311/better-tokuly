import { createFileRoute } from "@tanstack/react-router";
import { InterestsScreen } from "#/features/news/interests-screen";

export const Route = createFileRoute("/_app/news/interests")({ component: InterestsScreen });
