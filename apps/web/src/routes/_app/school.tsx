import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the school area: today (index), the timetable, studying, study rooms, exams and assignments.
export const Route = createFileRoute("/_app/school")({ component: Outlet });
