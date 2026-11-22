import { createFileRoute } from "@tanstack/react-router";
import { SubmitScreen } from "#/features/emoji/submit-screen";

export const Route = createFileRoute("/_app/emoji/new-emoji")({ component: SubmitScreen });
