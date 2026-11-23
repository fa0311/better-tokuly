import { createFileRoute } from "@tanstack/react-router";
import { CreateCommunityScreen } from "#/features/community/create-screen";

// `/communities/new`: start a community.
export const Route = createFileRoute("/_app/communities/new")({ component: CreateCommunityScreen });
