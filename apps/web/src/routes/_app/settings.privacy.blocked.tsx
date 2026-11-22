import { createFileRoute } from "@tanstack/react-router";
import { BlockedUsersScreen } from "#/features/settings/privacy/blocked-users-screen";

export const Route = createFileRoute("/_app/settings/privacy/blocked")({
  component: BlockedUsersScreen,
});
