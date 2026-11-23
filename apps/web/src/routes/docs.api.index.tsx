import { createFileRoute } from "@tanstack/react-router";
import { ReferenceHome } from "#/features/dev/reference/reference-screen";

export const Route = createFileRoute("/docs/api/")({ component: ReferenceHome });
