import { createFileRoute } from "@tanstack/react-router";
import { EditStreamScreen } from "#/features/live/studio/stream-form";

export const Route = createFileRoute("/_app/studio/live/$streamId/edit")({ component: EditPage });

function EditPage() {
  const { streamId } = Route.useParams();
  return <EditStreamScreen key={streamId} streamId={streamId} />;
}
