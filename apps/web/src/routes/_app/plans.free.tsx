import { createFileRoute } from "@tanstack/react-router";
import { FreeScreen } from "#/features/plans/free/free-screen";

// `/plans/free`: where the server points for "あそぼ!" and "a friend is free" notifications.
export const Route = createFileRoute("/_app/plans/free")({ component: FreeScreen });
