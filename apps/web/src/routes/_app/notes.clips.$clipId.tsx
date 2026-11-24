import { createFileRoute } from "@tanstack/react-router";
import { ClipPage } from "#/features/notes/clips-page";

// `/notes/clips/<id>`: the address the server uses for a clip.
export const Route = createFileRoute("/_app/notes/clips/$clipId")({ component: Clip });

function Clip() {
  const { clipId } = Route.useParams();
  return <ClipPage key={clipId} clipId={clipId} />;
}
