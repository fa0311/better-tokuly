import { createFileRoute } from "@tanstack/react-router";
import { MomentCameraScreen } from "#/features/moments/camera-screen";

export const Route = createFileRoute("/_app/moments/camera")({ component: MomentCameraScreen });
