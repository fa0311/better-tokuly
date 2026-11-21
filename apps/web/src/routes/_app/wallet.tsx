import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the wallet: its pages are the `wallet.*.tsx` routes.
export const Route = createFileRoute("/_app/wallet")({ component: Outlet });
