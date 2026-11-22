import { createFileRoute } from "@tanstack/react-router";
import { DetailScreen } from "#/features/emoji/detail-screen";

// `/emoji/<name>`: the address the server uses for an emoji in notifications, the feed and search.
// The area's own pages have names no emoji can have (a hyphen), except `/emoji/submissions`, which the server links to.
export const Route = createFileRoute("/_app/emoji/$name")({ component: DetailPage });

function DetailPage() {
  const { name } = Route.useParams();
  return <DetailScreen key={name} name={name} />;
}
