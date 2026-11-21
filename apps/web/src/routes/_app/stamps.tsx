import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the stamps area: its pages are the `stamps.*.tsx` routes.
export const Route = createFileRoute("/_app/stamps")({ component: Outlet });
