import { createFileRoute } from "@tanstack/react-router";
import { QuestHomeScreen } from "#/features/quest/home-screen";

export const Route = createFileRoute("/_app/quest/")({ component: QuestHomeScreen });
