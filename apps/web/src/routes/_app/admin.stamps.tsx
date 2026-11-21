import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/_app/admin/stamps")({ component: Page });

function Page() {
  return <div className="p-6 text-xl font-black">admin stamps</div>;
}
