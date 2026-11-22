import { createFileRoute } from "@tanstack/react-router";
import { CallHistoryScreen } from "#/features/chat/calls/call-history-screen";

export const Route = createFileRoute("/_app/chat/calls")({ component: CallHistoryScreen });
