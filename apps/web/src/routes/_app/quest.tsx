import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the quest area: its pages are the `quest.*.tsx` routes.
export const Route = createFileRoute("/_app/quest")({ component: Outlet });
