import { createFileRoute } from "@tanstack/react-router";
import { PromptsScreen } from "#/features/moments/admin/prompts-screen";

export const Route = createFileRoute("/_app/admin/moments")({ component: PromptsScreen });
