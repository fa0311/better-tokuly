import { createFileRoute } from "@tanstack/react-router";
import { ReactionSettingsScreen } from "#/features/engagement/reaction-settings-screen";

/** What people may react with on the caller's own content. */
export const Route = createFileRoute("/_app/settings/reactions")({
  component: ReactionSettingsScreen,
});
