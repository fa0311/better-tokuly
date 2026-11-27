export const RANGES = [7, 28, 90] as const;
export type RangeDays = (typeof RANGES)[number];

export interface AnalyticsSearch {
  /** One short's numbers instead of the channel's. */
  short?: string;
  days?: RangeDays;
}

export function validateAnalyticsSearch(search: Record<string, unknown>): AnalyticsSearch {
  const days = RANGES.find((value) => value === Number(search.days));
  return {
    ...(typeof search.short === "string" && search.short ? { short: search.short } : {}),
    ...(days && days !== 28 ? { days } : {}),
  };
}
