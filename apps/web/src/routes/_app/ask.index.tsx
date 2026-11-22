import { createFileRoute } from "@tanstack/react-router";
import { HubScreen } from "#/features/ask/hub-screen";

export const Route = createFileRoute("/_app/ask/")({ component: HubScreen });
