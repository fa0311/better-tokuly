import { createFileRoute } from "@tanstack/react-router";
import { CircleScreen } from "#/features/friends/circle-screen";

export const Route = createFileRoute("/_app/circles/$circleId")({ component: CirclePage });

function CirclePage() {
  const { circleId } = Route.useParams();
  return <CircleScreen key={circleId} circleId={circleId} />;
}
