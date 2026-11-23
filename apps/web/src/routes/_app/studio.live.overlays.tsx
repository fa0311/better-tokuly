import { createFileRoute } from "@tanstack/react-router";
import { OverlaysScreen } from "#/features/live/studio/overlays-screen";

export const Route = createFileRoute("/_app/studio/live/overlays")({ component: OverlaysScreen });
