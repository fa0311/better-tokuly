import { createFileRoute } from "@tanstack/react-router";
import { StoreScreen } from "#/features/stamps/store-screen";

export const Route = createFileRoute("/_app/stamps/")({ component: StoreScreen });
