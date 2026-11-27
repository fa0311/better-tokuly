export function parseClipsSearch(search: Record<string, unknown>): { user?: string } {
  return { user: typeof search.user === "string" && search.user ? search.user : undefined };
}
