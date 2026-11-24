import { createFileRoute } from "@tanstack/react-router";
import { CirclesScreen } from "#/features/friends/circles-screen";

export const Route = createFileRoute("/_app/circles/")({ component: CirclesScreen });
