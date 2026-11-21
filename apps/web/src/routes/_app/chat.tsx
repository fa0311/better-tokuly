import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/_app/chat")({
  validateSearch: (search): { user?: string } => ({ user: typeof search.user === "string" ? search.user : undefined }),
  component: Page,
});

function Page() {
  return <div className="p-6 text-xl font-black">chat</div>;
}
