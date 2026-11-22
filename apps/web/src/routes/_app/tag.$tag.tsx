import { createFileRoute } from "@tanstack/react-router";
import { TagScreen } from "#/features/notes/tag-screen";

export const Route = createFileRoute("/_app/tag/$tag")({ component: TagPage });

function TagPage() {
  const { tag } = Route.useParams();
  return <TagScreen key={tag} tag={tag} />;
}
