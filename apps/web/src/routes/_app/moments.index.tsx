import { createFileRoute } from "@tanstack/react-router";
import { MomentsTodayScreen } from "#/features/moments/today-screen";

export const Route = createFileRoute("/_app/moments/")({ component: MomentsTodayScreen });
