import { kindFromSlug } from "@tokuly/core/search";
import { useLocalSearchParams } from "expo-router";
import { SearchScreen } from "#/features/search/search-screen";

/** `/search`, or a link's `/search?q=…&kind=…` (shown, not remembered as the viewer's own search). */
export default function Search() {
  const { q, kind } = useLocalSearchParams<{ q?: string; kind?: string }>();
  return <SearchScreen start={{ query: q?.trim() || undefined, kind: kindFromSlug(kind) }} />;
}
