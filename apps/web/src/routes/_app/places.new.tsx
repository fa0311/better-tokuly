import { createFileRoute } from "@tanstack/react-router";
import { labelFromKey } from "#/features/places/labels";
import { NewPlaceScreen } from "#/features/places/place-editor-screen";

interface NewPlaceSearch {
  lat?: number;
  lng?: number;
  name?: string;
  label?: string;
}

const coordinate = (value: unknown, limit: number): number | undefined => {
  const number = typeof value === "number" ? value : typeof value === "string" ? Number(value) : Number.NaN;
  return Number.isFinite(number) && Math.abs(number) <= limit ? number : undefined;
};

// `/places/new?lat=…&lng=…&name=…&label=home|school|work`: a new place, optionally started from a spot.
export const Route = createFileRoute("/_app/places/new")({
  validateSearch: (search: Record<string, unknown>): NewPlaceSearch => {
    const lat = coordinate(search.lat, 90);
    const lng = coordinate(search.lng, 180);
    return {
      ...(lat !== undefined && lng !== undefined ? { lat, lng } : {}),
      ...(typeof search.name === "string" && search.name ? { name: search.name.slice(0, 60) } : {}),
      ...(typeof search.label === "string" && search.label ? { label: search.label } : {}),
    };
  },
  component: NewPlacePage,
});

function NewPlacePage() {
  const { lat, lng, name, label } = Route.useSearch();
  return <NewPlaceScreen draft={{ lat, lng, name, label: label ? labelFromKey(label) : undefined }} />;
}
