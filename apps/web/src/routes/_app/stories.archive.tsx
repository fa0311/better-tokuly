import { createFileRoute } from "@tanstack/react-router";
import { StoryArchiveScreen } from "#/features/stories/archive-screen";

export const Route = createFileRoute("/_app/stories/archive")({ component: StoryArchiveScreen });
