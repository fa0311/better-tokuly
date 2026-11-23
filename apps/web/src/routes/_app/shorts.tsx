import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the shorts area: the swipe feed (index, `$shortId`) and the `shorts.*.tsx` pages.
export const Route = createFileRoute("/_app/shorts")({ component: Outlet });
