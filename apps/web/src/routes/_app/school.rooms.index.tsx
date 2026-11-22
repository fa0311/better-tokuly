import { createFileRoute } from "@tanstack/react-router";
import { RoomsScreen } from "#/features/school/rooms-screen";

export const Route = createFileRoute("/_app/school/rooms/")({ component: RoomsScreen });
