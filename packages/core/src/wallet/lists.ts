/** Splits a newest-first list into runs that happened on the same local day. */
export function groupByDay<T>(
  items: readonly T[],
  dateOf: (item: T) => Date | undefined,
): { day: string; date: Date | undefined; items: T[] }[] {
  const groups: { day: string; date: Date | undefined; items: T[] }[] = [];
  for (const item of items) {
    const date = dateOf(item);
    const day = date ? `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}` : "";
    const last = groups[groups.length - 1];
    if (last && last.day === day) last.items.push(item);
    else groups.push({ day, date, items: [item] });
  }
  return groups;
}

/** Whole days between the local day of `date` and the local day of `now` (0 = today, 1 = yesterday). */
export function daysAgo(date: Date, now: Date): number {
  const start = (value: Date) =>
    new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
  return Math.round((start(now) - start(date)) / 86_400_000);
}

/** Rows of an infinite list, without the duplicates that a row pushed live can cause on the next page. */
export function uniqueById<T extends { id: string }>(items: T[]): T[] {
  const seen = new Set<string>();
  return items.filter((item) => {
    if (seen.has(item.id)) return false;
    seen.add(item.id);
    return true;
  });
}
