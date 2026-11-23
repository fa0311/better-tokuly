import { createFileRoute } from "@tanstack/react-router";
import { GiftHistoryScreen } from "#/features/live/gifts/history-screen";

// `/live/gifts`: gifts sent and received.
export const Route = createFileRoute("/_app/live/gifts")({ component: GiftHistoryScreen });
