import { createFileRoute } from "@tanstack/react-router";
import { QuestSettingsScreen } from "#/features/quest/settings-screen";

export const Route = createFileRoute("/_app/quest/settings")({ component: QuestSettingsScreen });
