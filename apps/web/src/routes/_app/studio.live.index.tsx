import { createFileRoute } from "@tanstack/react-router";
import { LiveStudioHome } from "#/features/live/studio/studio-home";

export const Route = createFileRoute("/_app/studio/live/")({ component: LiveStudioHome });
