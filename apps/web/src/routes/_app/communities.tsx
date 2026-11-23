import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the communities area: the hub (index) and the `communities.*.tsx` pages.
export const Route = createFileRoute("/_app/communities")({ component: Outlet });
