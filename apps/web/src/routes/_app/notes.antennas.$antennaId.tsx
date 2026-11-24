import { createFileRoute } from "@tanstack/react-router";
import { AntennaScreen } from "#/features/notes/antennas-screen";

// `/notes/antennas/<id>`: the address the server uses for an antenna (notifications).
export const Route = createFileRoute("/_app/notes/antennas/$antennaId")({ component: AntennaPage });

function AntennaPage() {
  const { antennaId } = Route.useParams();
  return <AntennaScreen key={antennaId} antennaId={antennaId} />;
}
