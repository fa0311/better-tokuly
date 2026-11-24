import { createFileRoute } from "@tanstack/react-router";
import { ListScreen } from "#/features/notes/lists-screen";

// `/notes/lists/<id>`: the address the server uses for a list (notifications).
export const Route = createFileRoute("/_app/notes/lists/$listId")({ component: ListPage });

function ListPage() {
  const { listId } = Route.useParams();
  return <ListScreen key={listId} listId={listId} />;
}
