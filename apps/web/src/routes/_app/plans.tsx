import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the plans area ("あそぶ"): the hub (index) and the `plans.*.tsx` pages.
export const Route = createFileRoute("/_app/plans")({ component: Outlet });
