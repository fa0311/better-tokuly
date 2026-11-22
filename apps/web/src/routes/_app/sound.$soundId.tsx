import { createFileRoute } from "@tanstack/react-router";
import { SoundScreen } from "#/features/shorts/sound-screen";

export const Route = createFileRoute("/_app/sound/$soundId")({ component: SoundPage });

function SoundPage() {
  const { soundId } = Route.useParams();
  return <SoundScreen key={soundId} soundId={soundId} />;
}
