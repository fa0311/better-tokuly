import { createFileRoute } from "@tanstack/react-router";
import { AppScreen } from "#/features/pwa/app-screen";

// "アプリとして使う": reached from the settings list.
export const Route = createFileRoute("/_app/pwa")({ component: AppScreen });
