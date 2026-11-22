import { createFileRoute } from "@tanstack/react-router";
import { ExamsScreen } from "#/features/school/exams-screen";

export const Route = createFileRoute("/_app/school/exams/")({ component: ExamsScreen });
