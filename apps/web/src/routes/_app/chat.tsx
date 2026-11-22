import { createFileRoute } from "@tanstack/react-router";
import { ChatLayout } from "#/features/chat/chat-layout";

// Layout of the chat area: the list, with the open conversation (`chat.$conversationId.tsx`) next to it.
// `?user=<handle or user ID>` opens the direct conversation with that person.
export const Route = createFileRoute("/_app/chat")({
  validateSearch: (search): { user?: string } => ({
    user: typeof search.user === "string" ? search.user : undefined,
  }),
  component: Page,
});

function Page() {
  const { user } = Route.useSearch();
  return <ChatLayout openUser={user} />;
}
