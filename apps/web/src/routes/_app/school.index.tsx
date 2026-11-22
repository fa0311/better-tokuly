import { createFileRoute } from "@tanstack/react-router";
import { TodayScreen } from "#/features/school/today-screen";

export const Route = createFileRoute("/_app/school/")({ component: TodayScreen });
