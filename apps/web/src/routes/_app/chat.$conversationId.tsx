import { createFileRoute } from "@tanstack/react-router";
import { ConversationScreen } from "#/features/chat/conversation-screen";

// `?message=<id>` opens the conversation at that message (search hits, reports, notifications).
export const Route = createFileRoute("/_app/chat/$conversationId")({
  validateSearch: (search): { message?: string } => ({
    message: typeof search.message === "string" ? search.message : undefined,
  }),
  component: Page,
});

function Page() {
  const { conversationId } = Route.useParams();
  const { message } = Route.useSearch();
  return (
    <ConversationScreen
      key={conversationId}
      conversationId={conversationId}
      openAtMessageId={message}
    />
  );
}
