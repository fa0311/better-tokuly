import { createFileRoute } from "@tanstack/react-router";
import { AiSettingsScreen } from "#/features/chat/ai-settings-screen";

export const Route = createFileRoute("/_app/settings/chat-ai")({ component: AiSettingsScreen });
