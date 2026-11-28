import { createFileRoute } from "@tanstack/react-router";
import { ReviewScreen } from "#/features/miniapps/review-screen";

export const Route = createFileRoute("/_app/admin/mini-apps")({ component: ReviewScreen });
