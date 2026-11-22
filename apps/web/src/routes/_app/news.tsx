import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the news area: the list (index) and the `news.*.tsx` pages.
export const Route = createFileRoute("/_app/news")({ component: Outlet });
