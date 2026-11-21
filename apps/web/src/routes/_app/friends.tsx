import { createFileRoute } from "@tanstack/react-router";
import { FriendsScreen } from "#/features/friends/friends-screen";

export const Route = createFileRoute("/_app/friends")({ component: FriendsScreen });
