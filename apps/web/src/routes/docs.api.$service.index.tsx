import { createFileRoute } from "@tanstack/react-router";
import { ReferenceService } from "#/features/dev/reference/reference-screen";

// `/docs/api/tokuly.notes.v1.NoteService`
export const Route = createFileRoute("/docs/api/$service/")({ component: ServicePage });

function ServicePage() {
  const { service } = Route.useParams();
  return <ReferenceService service={service} />;
}
