import { createFileRoute } from "@tanstack/react-router";
import { referenceHead, serviceShortName } from "#/features/dev/reference/head";
import { ReferenceService } from "#/features/dev/reference/reference-screen";

// `/docs/api/tokuly.notes.v1.NoteService`
export const Route = createFileRoute("/docs/api/$service/")({
  head: ({ params }) =>
    referenceHead(serviceShortName(params.service), {
      description: `${params.service}: every method of the service in the Tokuly API reference.`,
    }),
  component: ServicePage,
});

function ServicePage() {
  const { service } = Route.useParams();
  return <ReferenceService service={service} />;
}
