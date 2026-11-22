import { createFileRoute } from "@tanstack/react-router";
import { AssignmentsScreen } from "#/features/school/exams-screen";

// `/school/assignments`: where the assignment reminders lead.
export const Route = createFileRoute("/_app/school/assignments")({ component: AssignmentsScreen });
