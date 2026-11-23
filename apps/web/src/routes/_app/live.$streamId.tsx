import { createFileRoute } from "@tanstack/react-router";
import { WatchLiveScreen } from "#/features/live/watch-screen";

// `/live/<id>`: a stream (UUID or short ID) with its chat.
export const Route = createFileRoute("/_app/live/$streamId")({ component: WatchPage });

function WatchPage() {
  const { streamId } = Route.useParams();
  return <WatchLiveScreen key={streamId} streamId={streamId} />;
}
