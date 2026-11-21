import { createFileRoute } from "@tanstack/react-router";
import { ConsoleLayout } from "#/features/admin/console-layout";

// Layout of the staff console: its pages are the `admin.*.tsx` routes.
export const Route = createFileRoute("/_app/admin")({ component: ConsoleLayout });
