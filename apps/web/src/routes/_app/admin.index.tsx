import { createFileRoute } from "@tanstack/react-router";
import { DashboardScreen } from "#/features/admin/dashboard/dashboard-screen";

export const Route = createFileRoute("/_app/admin/")({ component: DashboardScreen });
