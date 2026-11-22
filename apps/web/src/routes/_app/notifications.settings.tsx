import { createFileRoute } from "@tanstack/react-router";
import { NotificationSettingsScreen } from "#/features/notifications/settings-screen";

export const Route = createFileRoute("/_app/notifications/settings")({
  component: NotificationSettingsScreen,
});
