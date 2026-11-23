import { createFileRoute } from "@tanstack/react-router";
import { NewStreamScreen } from "#/features/live/studio/stream-form";

export const Route = createFileRoute("/_app/studio/live/new")({ component: NewStreamScreen });
