import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the custom emoji area: its pages are the `emoji.*.tsx` routes.
export const Route = createFileRoute("/_app/emoji")({ component: Outlet });
