import { createFileRoute } from "@tanstack/react-router";
import { StudyScreen } from "#/features/school/study-screen";

export const Route = createFileRoute("/_app/school/study")({ component: StudyScreen });
