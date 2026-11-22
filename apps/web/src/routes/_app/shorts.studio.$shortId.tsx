import { createFileRoute } from "@tanstack/react-router";
import { EditScreen } from "#/features/shorts/studio/edit-screen";

export const Route = createFileRoute("/_app/shorts/studio/$shortId")({ component: EditPage });

function EditPage() {
  const { shortId } = Route.useParams();
  return <EditScreen key={shortId} shortId={shortId} />;
}
