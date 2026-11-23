import { createFileRoute } from "@tanstack/react-router";
import { QuestAdminScreen } from "#/features/quest/admin-screen";

// Staff: badges given and taken by hand. Linked from the staff console.
export const Route = createFileRoute("/_app/quest/admin")({ component: QuestAdminScreen });
