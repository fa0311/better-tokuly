import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the creator studio: its pages are the `studio.*.tsx` routes.
export const Route = createFileRoute("/_app/studio")({ component: Outlet });
