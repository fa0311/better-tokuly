import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the notes area: the timelines (index) and the `notes.*.tsx` pages.
export const Route = createFileRoute("/_app/notes")({ component: Outlet });
