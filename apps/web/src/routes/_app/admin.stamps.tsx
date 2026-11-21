import { createFileRoute } from "@tanstack/react-router";
import { ReviewScreen } from "#/features/stamps/review-screen";

export const Route = createFileRoute("/_app/admin/stamps")({ component: ReviewScreen });
