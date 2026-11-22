import { createFileRoute } from "@tanstack/react-router";
import { SubmissionsScreen } from "#/features/emoji/submissions-screen";

// `/emoji/submissions`: where the server sends authors from "declined" and "disabled" notifications.
export const Route = createFileRoute("/_app/emoji/submissions/")({ component: SubmissionsScreen });
