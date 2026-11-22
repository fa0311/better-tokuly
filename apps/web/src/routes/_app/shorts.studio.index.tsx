import { createFileRoute } from "@tanstack/react-router";
import { MyShortsScreen } from "#/features/shorts/studio/my-shorts-screen";

export const Route = createFileRoute("/_app/shorts/studio/")({ component: MyShortsScreen });
