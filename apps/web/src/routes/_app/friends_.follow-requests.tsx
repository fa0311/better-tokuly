import { createFileRoute } from "@tanstack/react-router";
import { FollowRequestsScreen } from "#/features/friends/follow-requests";

/** Where the follow-request notification leads. */
export const Route = createFileRoute("/_app/friends_/follow-requests")({
  component: FollowRequestsScreen,
});
