import { createFileRoute } from "@tanstack/react-router";
import { RoomScreen } from "#/features/school/room-screen";

// `/school/rooms/<id>`: the address the server uses for a study room in invitations and reports.
export const Route = createFileRoute("/_app/school/rooms/$roomId")({ component: RoomPage });

function RoomPage() {
  const { roomId } = Route.useParams();
  return <RoomScreen key={roomId} roomId={roomId} />;
}
