import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/_app/notes")({ component: Page });

function Page() {
  return <div className="p-6 text-xl font-black">notes</div>;
}
