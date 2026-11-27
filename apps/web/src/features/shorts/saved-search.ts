export function validateSavedSearch(search: Record<string, unknown>): { tab?: "liked" } {
  return search.tab === "liked" ? { tab: "liked" } : {};
}
