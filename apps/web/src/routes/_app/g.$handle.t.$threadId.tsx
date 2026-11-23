import { createFileRoute } from "@tanstack/react-router";
import { ThreadScreen } from "#/features/community/thread-screen";

/**
 * `/g/<handle>/t/<thread>`: a thread and its replies. `<thread>` is its UUID or its short ID;
 * `?reply=<id>` (a notification's link) brings that reply into view.
 */
export const Route = createFileRoute("/_app/g/$handle/t/$threadId")({
  validateSearch: (search: Record<string, unknown>): { reply?: string } =>
    typeof search.reply === "string" && search.reply ? { reply: search.reply } : {},
  component: ThreadPage,
});

function ThreadPage() {
  const { threadId } = Route.useParams();
  const { reply } = Route.useSearch();
  return <ThreadScreen key={threadId} threadId={threadId} focusReplyId={reply} />;
}
