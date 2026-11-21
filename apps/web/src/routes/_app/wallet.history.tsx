import { createFileRoute } from "@tanstack/react-router";
import { HistoryScreen } from "#/features/wallet/history-screen";

export const Route = createFileRoute("/_app/wallet/history")({ component: HistoryScreen });
