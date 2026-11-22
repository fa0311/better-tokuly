import { createFileRoute } from "@tanstack/react-router";
import { PromptsScreen } from "#/features/ask/prompts-screen";

export const Route = createFileRoute("/_app/ask/prompts/")({ component: PromptsScreen });
