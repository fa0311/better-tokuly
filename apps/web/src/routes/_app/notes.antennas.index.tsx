import { createFileRoute } from "@tanstack/react-router";
import { AntennasScreen } from "#/features/notes/antennas-screen";

// `/notes/antennas`: the viewer's antennas.
export const Route = createFileRoute("/_app/notes/antennas/")({ component: AntennasScreen });
