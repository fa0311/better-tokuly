import { createFileRoute } from "@tanstack/react-router";
import { FollowListsScreen } from "#/features/friends/follow-lists-screen";

export const Route = createFileRoute("/_app/u_/$handle/following")({ component: FollowListPage });

function FollowListPage() {
  const { handle } = Route.useParams();
  return <FollowListsScreen key={handle} handle={handle} tab="following" />;
}
