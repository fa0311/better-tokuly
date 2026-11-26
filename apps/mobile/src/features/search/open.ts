import { search } from "@tokuly/api";
import { extraText, hitExtra, hitPath } from "@tokuly/core/search";
import { router } from "expo-router";
import { webScreenHref } from "#/features/web/handoff";
import { screenFor } from "#/seams/links";

/**
 * What a tap on a result does. A hashtag is searched for here (`#tag` finds
 * every kind that carries it); everything else opens where its path leads in
 * the app, or on the web screen when the app has no screen for it yet; a hit
 * without a usable path is searched for by its title.
 */
export function openHit(hit: search.SearchHit, searchFor: (query: string) => void): void {
  const tag =
    hit.kind === search.SearchKind.HASHTAG &&
    (extraText(hitExtra(hit), "tag") || hit.title.replace(/^[#＃]+/, ""));
  const path = hitPath(hit);
  if (tag) searchFor(`#${tag}`);
  else if (path) router.push(screenFor(path) ?? webScreenHref(path));
  else searchFor(hit.title || hit.id);
}
