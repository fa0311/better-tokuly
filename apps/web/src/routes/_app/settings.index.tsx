import { createFileRoute } from "@tanstack/react-router";
import { SettingsHome } from "#/features/settings/shell/settings-home";

export const Route = createFileRoute("/_app/settings/")({ component: SettingsHome });
