import { defaultStringifySearch } from "@tanstack/react-router";
import type { search } from "@tokuly/api";
import { hitPath, isKind, type Kind, kindSlug, type Sort } from "@tokuly/core/search";
import type { SearchParams } from "./params";

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

/**
 * The address of a result page: `/search?q=…&kind=…`. Written with the
 * router's own search serialisation, so the router reads back exactly what
 * was put in: a quoted phrase, a number or `{…}` stay the text they are.
 */
export function searchHref(
  query: string,
  options: { kind?: Kind; sort?: Sort; owner?: string } = {},
): string {
  const params: SearchParams = {
    q: query.trim(),
    kind: options.kind === undefined ? undefined : kindSlug(options.kind),
    sort: options.sort,
    owner: options.owner || undefined,
  };
  return `/search${defaultStringifySearch(params)}`;
}
