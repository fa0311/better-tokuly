import { createFileRoute } from "@tanstack/react-router";
import { PersonScreen } from "#/features/quest/person-screen";

// `/quest/u/<handle>`: what a person shows of their quest progress.
export const Route = createFileRoute("/_app/quest/u/$handle")({ component: PersonPage });

function PersonPage() {
  const { handle } = Route.useParams();
  return <PersonScreen key={handle} handle={handle} />;
}
