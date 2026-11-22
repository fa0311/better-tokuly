import { createFileRoute } from "@tanstack/react-router";
import { AnalyticsScreen } from "#/features/video/studio/analytics-screen";

export const Route = createFileRoute("/_app/studio/analytics")({ component: AnalyticsScreen });
