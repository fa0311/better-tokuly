import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the developer platform: its pages are the `developers.*.tsx` routes.
export const Route = createFileRoute("/_app/developers")({ component: Outlet });
