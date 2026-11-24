import { createFileRoute } from "@tanstack/react-router";
import { FollowListsScreen } from "#/features/friends/follow-lists-screen";

export const Route = createFileRoute("/_app/u_/$handle/followers")({ component: FollowListPage });

function FollowListPage() {
  const { handle } = Route.useParams();
  return <FollowListsScreen key={handle} handle={handle} tab="followers" />;
}
