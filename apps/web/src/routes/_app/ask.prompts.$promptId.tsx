import { createFileRoute } from "@tanstack/react-router";
import { PromptScreen } from "#/features/ask/prompt-screen";

/** `/ask/prompts/<id>`: a prompt's page (`ask.prompt` and `ask.prompt_response` notifications lead here). */
export const Route = createFileRoute("/_app/ask/prompts/$promptId")({ component: PromptPage });

function PromptPage() {
  const { promptId } = Route.useParams();
  return <PromptScreen key={promptId} promptId={promptId} />;
}
