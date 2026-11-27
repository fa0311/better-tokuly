export const SECTIONS = ["popular", "new", "featured", "official"] as const;
export const PRICES = ["all", "free", "paid"] as const;
export type Section = (typeof SECTIONS)[number];
export type Price = (typeof PRICES)[number];

/** What the browse page shows, as kept in the address: a search, or a shelf with filters. */
export interface BrowseSearch {
  q?: string;
  section?: Section;
  tag?: string;
  price?: Exclude<Price, "all">;
}

/** Reads the browse page's address parameters, ignoring anything unknown. */
export function validateBrowseSearch(search: Record<string, unknown>): BrowseSearch {
  const text = (value: unknown) =>
    typeof value === "string" && value.trim() ? value.trim().slice(0, 64) : undefined;
  return {
    q: text(search.q),
    section: SECTIONS.find((section) => section === search.section),
    tag: text(search.tag)?.replace(/^#+/, "").toLowerCase() || undefined,
    price: search.price === "free" || search.price === "paid" ? search.price : undefined,
  };
}
