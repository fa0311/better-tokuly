import { createFileRoute } from "@tanstack/react-router";
import { ModerationScreen } from "#/features/live/studio/moderation-screen";

export const Route = createFileRoute("/_app/studio/live/moderation")({
  component: ModerationScreen,
});
