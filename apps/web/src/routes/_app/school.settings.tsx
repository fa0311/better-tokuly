import { createFileRoute } from "@tanstack/react-router";
import { SchoolSettingsScreen } from "#/features/school/settings-screen";

export const Route = createFileRoute("/_app/school/settings")({ component: SchoolSettingsScreen });
