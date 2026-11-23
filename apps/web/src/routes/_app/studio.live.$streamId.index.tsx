import { createFileRoute } from "@tanstack/react-router";
import { ControlRoomScreen } from "#/features/live/studio/control-room";

// `/studio/live/<id>`: the control room of a stream of the caller.
export const Route = createFileRoute("/_app/studio/live/$streamId/")({ component: ControlPage });

function ControlPage() {
  const { streamId } = Route.useParams();
  return <ControlRoomScreen key={streamId} streamId={streamId} />;
}
