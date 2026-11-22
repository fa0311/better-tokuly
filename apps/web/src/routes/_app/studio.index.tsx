import { createFileRoute } from "@tanstack/react-router";
import { StudioHome } from "#/features/video/studio/studio-home";

export const Route = createFileRoute("/_app/studio/")({ component: StudioHome });
