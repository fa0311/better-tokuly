import { createFileRoute } from "@tanstack/react-router";
import { SettingsScreen } from "#/features/ask/settings-screen";

export const Route = createFileRoute("/_app/ask/settings")({ component: SettingsScreen });
