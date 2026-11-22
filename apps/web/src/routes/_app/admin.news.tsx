import { createFileRoute } from "@tanstack/react-router";
import { AdminNewsScreen } from "#/features/news/admin/admin-news-screen";

export const Route = createFileRoute("/_app/admin/news")({ component: AdminNewsScreen });
