import { createFileRoute } from "@tanstack/react-router";
import { SpaceScreen } from "#/features/spaces/room/space-screen";

// `/spaces/<id>`: the address the server uses for a room in notifications, search and reports.
export const Route = createFileRoute("/_app/spaces/$spaceId")({ component: SpacePage });

function SpacePage() {
  const { spaceId } = Route.useParams();
  return <SpaceScreen key={spaceId} spaceId={spaceId} />;
}
