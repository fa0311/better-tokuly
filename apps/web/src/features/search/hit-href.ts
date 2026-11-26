import type { search } from "@tokuly/api";
import { hitPath, isKind } from "@tokuly/core/search";
import { searchHref } from "./params";

/**
 * Where a result leads in this app (`hitPath`); a hit without a usable path
 * is searched for in its own tab.
 */
export function hitHref(
  hit: Pick<search.SearchHit, "kind" | "id" | "path" | "title" | "extraJson" | "owner">,
): string {
  return (
    hitPath(hit) ??
    searchHref(hit.title || hit.id, { kind: isKind(hit.kind) ? hit.kind : undefined })
  );
}
