import { createFileRoute } from "@tanstack/react-router";
import { ManageScreen } from "#/features/community/manage/manage-screen";

/** `/g/<handle>/manage`: the staff's way in to settings, members' requests, invites and safety. */
export const Route = createFileRoute("/_app/g/$handle/manage/")({ component: ManageScreen });
