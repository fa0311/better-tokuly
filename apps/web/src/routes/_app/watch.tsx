import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the video area: the feeds (index), the watch page and the `watch.*.tsx` pages.
export const Route = createFileRoute("/_app/watch")({ component: Outlet });
