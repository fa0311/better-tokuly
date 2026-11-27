import { createFileRoute } from "@tanstack/react-router";
import { CreateScreen } from "#/features/shorts/studio/create-screen";
import { validateCreateSearch } from "#/features/shorts/studio/create-search";

/** Post a short. `?sound=`, `?remix=&kind=`, `?clip=` and `?tag=` start it from something. */
export const Route = createFileRoute("/_app/shorts/new")({
  validateSearch: validateCreateSearch,
  component: CreatePage,
});

function CreatePage() {
  const search = Route.useSearch();
  return <CreateScreen search={search} />;
}
