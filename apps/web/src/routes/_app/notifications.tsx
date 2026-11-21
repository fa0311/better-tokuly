import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the notifications area: the inbox (index) and the `notifications.*.tsx` pages.
export const Route = createFileRoute("/_app/notifications")({ component: Outlet });
