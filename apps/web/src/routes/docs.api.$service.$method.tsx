import { createFileRoute } from "@tanstack/react-router";
import { serviceShortName } from "#/features/dev/reference/data";
import { referenceHead } from "#/features/dev/reference/head";
import { ReferenceMethod } from "#/features/dev/reference/reference-screen";

// `/docs/api/tokuly.notes.v1.NoteService/CreateNote`: the same two parts as the method's path.
export const Route = createFileRoute("/docs/api/$service/$method")({
  // Named from the address, so links and previews say which method they lead to before the
  // page has asked the API for it.
  head: ({ params }) =>
    referenceHead(`${params.method} — ${serviceShortName(params.service)}`, {
      description: `/${params.service}/${params.method}: request, response, scopes and examples in the Tokuly API reference.`,
    }),
  component: MethodPage,
});

function MethodPage() {
  const { service, method } = Route.useParams();
  return <ReferenceMethod key={`${service}/${method}`} service={service} method={method} />;
}
