import { createFileRoute } from "@tanstack/react-router";
import { SecuritySection } from "#/features/settings/security/security-section";

export const Route = createFileRoute("/_app/settings/security/")({ component: SecuritySection });
