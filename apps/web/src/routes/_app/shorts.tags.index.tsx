import { createFileRoute } from "@tanstack/react-router";
import { TrendingTagsScreen } from "#/features/shorts/hashtag-screen";

export const Route = createFileRoute("/_app/shorts/tags/")({ component: TrendingTagsScreen });
