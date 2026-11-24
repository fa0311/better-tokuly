import { createFileRoute } from "@tanstack/react-router";
import { DraftsScreen } from "#/features/notes/drafts-screen";

export const Route = createFileRoute("/_app/notes/drafts")({ component: DraftsScreen });
