import { createFileRoute } from "@tanstack/react-router";
import { AiAdminScreen } from "#/features/chat/ai-admin-screen";

export const Route = createFileRoute("/_app/admin/chat-ai")({ component: AiAdminScreen });
