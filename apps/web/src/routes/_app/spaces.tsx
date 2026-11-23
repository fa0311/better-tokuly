import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the audio rooms area: discovery (index), a room, a hangout.
export const Route = createFileRoute("/_app/spaces")({ component: Outlet });
