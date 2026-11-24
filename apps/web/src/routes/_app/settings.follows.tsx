import { createFileRoute } from "@tanstack/react-router";
import { FollowSettingsScreen } from "#/features/friends/follow-settings-screen";

export const Route = createFileRoute("/_app/settings/follows")({ component: FollowSettingsScreen });
