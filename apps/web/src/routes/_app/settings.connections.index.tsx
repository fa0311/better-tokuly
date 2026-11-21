import { createFileRoute } from "@tanstack/react-router";
import { ConnectionsSection } from "#/features/settings/connections/connections-section";

export const Route = createFileRoute("/_app/settings/connections/")({ component: ConnectionsSection });
