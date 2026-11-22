import { createFileRoute } from "@tanstack/react-router";
import { SubscriptionsScreen } from "#/features/video/subscriptions-screen";

export const Route = createFileRoute("/_app/watch/channels")({ component: SubscriptionsScreen });
