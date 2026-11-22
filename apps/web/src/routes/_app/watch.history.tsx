import { createFileRoute } from "@tanstack/react-router";
import { HistoryScreen } from "#/features/video/history-screen";

export const Route = createFileRoute("/_app/watch/history")({ component: HistoryScreen });
