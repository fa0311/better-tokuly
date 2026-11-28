import { createFileRoute } from "@tanstack/react-router";
import { MomentCalendarScreen } from "#/features/moments/calendar-screen";

export const Route = createFileRoute("/_app/moments/calendar")({ component: MomentCalendarScreen });
