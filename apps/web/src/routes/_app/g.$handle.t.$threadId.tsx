import { createFileRoute } from "@tanstack/react-router";
import { ThreadScreen } from "#/features/community/thread-screen";

/** `/g/<handle>/t/<thread>`: a thread and its replies. `<thread>` is its UUID or its short ID. */
export const Route = createFileRoute("/_app/g/$handle/t/$threadId")({ component: ThreadPage });

function ThreadPage() {
  const { threadId } = Route.useParams();
  return <ThreadScreen key={threadId} threadId={threadId} />;
}
