import { createFileRoute } from "@tanstack/react-router";
import { SecurityLogScreen } from "#/features/settings/security/security-log";

export const Route = createFileRoute("/_app/settings/security/log")({ component: SecurityLogScreen });
