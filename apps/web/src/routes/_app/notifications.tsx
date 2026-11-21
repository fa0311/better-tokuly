import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/_app/notifications")({ component: Page });

function Page() {
  return <div className="p-6 text-xl font-black">notifications</div>;
}
