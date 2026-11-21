import { createFileRoute } from "@tanstack/react-router";
import { DataSection } from "#/features/settings/your-data/data-section";

export const Route = createFileRoute("/_app/settings/data")({ component: DataSection });
