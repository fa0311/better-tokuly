import { createFileRoute } from "@tanstack/react-router";
import { SettingsLayout } from "#/features/settings/shell/settings-layout";

// Layout of the settings: its pages are the `settings.*.tsx` routes.
export const Route = createFileRoute("/_app/settings")({ component: SettingsLayout });
