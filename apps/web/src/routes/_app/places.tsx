import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the places area: its pages are the `places.*.tsx` routes.
export const Route = createFileRoute("/_app/places")({ component: Outlet });
