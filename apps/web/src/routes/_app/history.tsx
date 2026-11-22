import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the history area: its pages are the `history.*.tsx` routes.
export const Route = createFileRoute("/_app/history")({ component: Outlet });
