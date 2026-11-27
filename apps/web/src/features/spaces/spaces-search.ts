export type SpacesTab = "discover" | "hangouts" | "mine";

export interface SpacesSearch {
  tab?: Exclude<SpacesTab, "discover">;
  tag?: string;
}

/** `?tab=hangouts|mine` selects a tab and `?tag=` filters the public rooms. */
export function parseSpacesSearch(search: Record<string, unknown>): SpacesSearch {
  const tag = typeof search.tag === "string" ? search.tag.replace(/^#/, "").trim() : "";
  return {
    ...(search.tab === "hangouts" || search.tab === "mine" ? { tab: search.tab } : {}),
    ...(tag ? { tag } : {}),
  };
}
