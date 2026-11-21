import { createFileRoute } from "@tanstack/react-router";
import { MeScreen } from "#/features/me/me-screen";

export const Route = createFileRoute("/_app/me")({ component: MeScreen });
