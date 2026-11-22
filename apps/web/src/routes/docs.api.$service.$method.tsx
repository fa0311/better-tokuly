import { createFileRoute } from "@tanstack/react-router";
import { ReferenceMethod } from "#/features/dev/reference/reference-screen";

// `/docs/api/tokuly.notes.v1.NoteService/CreateNote`: the same two parts as the method's path.
export const Route = createFileRoute("/docs/api/$service/$method")({ component: MethodPage });

function MethodPage() {
  const { service, method } = Route.useParams();
  return <ReferenceMethod key={`${service}/${method}`} service={service} method={method} />;
}
