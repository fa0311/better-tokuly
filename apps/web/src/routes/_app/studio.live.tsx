import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the live part of the creator studio: its pages are the `studio.live.*.tsx` routes.
export const Route = createFileRoute("/_app/studio/live")({ component: Outlet });
