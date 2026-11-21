import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/_app/stamps")({ component: Page });

function Page() {
  return <div className="p-6 text-xl font-black">stamps</div>;
}
