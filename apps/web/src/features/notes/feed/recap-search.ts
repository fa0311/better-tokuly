/**
 * Which recap `/recap?p=…` shows: `2026-10` (a month), `2026` (a year);
 * absent = this month. A year is kept as a number, which is how the router
 * reads and writes a bare `2026` in the address.
 */
export interface RecapSearch {
  p?: string | number;
}

export function parseRecapSearch(search: Record<string, unknown>): RecapSearch {
  const p = search.p;
  if (typeof p === "number" && Number.isInteger(p) && p >= 2000 && p <= 9999) return { p };
  if (typeof p !== "string") return {};
  if (/^\d{4}$/.test(p)) return { p: Number(p) };
  return /^\d{4}-(0[1-9]|1[0-2])$/.test(p) ? { p } : {};
}
