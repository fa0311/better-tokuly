import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/_app/settings")({ component: Page });

function Page() {
  return <div className="p-6 text-xl font-black">settings</div>;
}
