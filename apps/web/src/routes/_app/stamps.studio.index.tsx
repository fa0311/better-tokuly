import { createFileRoute } from "@tanstack/react-router";
import { StudioScreen } from "#/features/stamps/studio-screen";

export const Route = createFileRoute("/_app/stamps/studio/")({ component: StudioScreen });
