import { createFileRoute } from "@tanstack/react-router";
import { AuthorizedAppsScreen } from "#/features/dev/authorized";

export const Route = createFileRoute("/_app/developers/authorized")({
  component: AuthorizedAppsScreen,
});
