import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the live area: browsing (index), a stream's page and the `live.*.tsx` pages.
export const Route = createFileRoute("/_app/live")({ component: Outlet });
