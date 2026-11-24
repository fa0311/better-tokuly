import { createFileRoute } from "@tanstack/react-router";
import { ListsScreen } from "#/features/notes/lists-screen";

// `/notes/lists`: the viewer's note lists, followed lists and the lists they are on.
export const Route = createFileRoute("/_app/notes/lists/")({ component: ListsScreen });
