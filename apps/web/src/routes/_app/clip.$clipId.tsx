import { createFileRoute } from "@tanstack/react-router";
import { ClipScreen } from "#/features/live/clips/clip-screen";

// `/clip/<id>`: a clip (UUID or short ID). The path is the one the API hands out for clips.
export const Route = createFileRoute("/_app/clip/$clipId")({ component: ClipPage });

function ClipPage() {
  const { clipId } = Route.useParams();
  return <ClipScreen key={clipId} clipId={clipId} />;
}
