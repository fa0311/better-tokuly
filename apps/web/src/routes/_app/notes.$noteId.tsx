import { createFileRoute } from "@tanstack/react-router";
import { NoteScreen } from "#/features/notes/note-screen";

// `/notes/<id>`: the address the server uses for a note in notifications, the feed, search and share links.
export const Route = createFileRoute("/_app/notes/$noteId")({ component: NotePage });

function NotePage() {
  const { noteId } = Route.useParams();
  return <NoteScreen key={noteId} noteId={noteId} />;
}
