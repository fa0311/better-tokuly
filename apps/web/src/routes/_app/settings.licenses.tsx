import { createFileRoute } from "@tanstack/react-router";
import { LicensesScreen } from "#/features/licenses/licenses-screen";

export const Route = createFileRoute("/_app/settings/licenses")({ component: LicensesScreen });
