/**
 * The game around footprints. The server only knows cells and percentages;
 * levels are how the app turns those into something to chase.
 */

/** Cells needed to reach a level: 5 for Lv.2, 20 for Lv.3, 45 for Lv.4 … (5·(n−1)²). */
export function cellsForLevel(level: number): number {
  return 5 * Math.max(0, level - 1) ** 2;
}

export interface ExplorerLevel {
  level: number;
  /** Cells at which this level began and at which the next one begins. */
  from: number;
  next: number;
  /** 0..1 of the way to the next level. */
  progress: number;
  /** Key of the title in the strings (`titles.<key>`). */
  title: "beginner" | "neighbour" | "townie" | "traveller" | "adventurer" | "legend";
}

/** The explorer level for a total number of uncovered cells. */
export function explorerLevel(cellCount: number): ExplorerLevel {
  const cells = Math.max(0, cellCount);
  const level = Math.floor(Math.sqrt(cells / 5)) + 1;
  const from = cellsForLevel(level);
  const next = cellsForLevel(level + 1);
  const title = level <= 2 ? "beginner" : level <= 5 ? "neighbour" : level <= 10 ? "townie" : level <= 20 ? "traveller" : level <= 35 ? "adventurer" : "legend";
  return { level, from, next, progress: (cells - from) / (next - from), title };
}

/** Percent explored at which a region levels up. A region is tiny next to a country, so the steps start small. */
export const REGION_MILESTONES = [0.001, 0.01, 0.05, 0.1, 0.25, 0.5, 1, 2, 5, 10, 20, 35, 50, 75, 100] as const;

export interface RegionLevel {
  /** 0 before the first milestone. */
  level: number;
  /** The milestone being worked towards; 100 once everything is explored. */
  nextPercent: number;
  /** 0..1 of the way from the last milestone to the next. */
  progress: number;
}

/** Where a region stands between its milestones. */
export function regionLevel(percent: number): RegionLevel {
  const reached = REGION_MILESTONES.filter((milestone) => percent >= milestone).length;
  if (reached === REGION_MILESTONES.length) return { level: reached, nextPercent: 100, progress: 1 };
  const previous = reached === 0 ? 0 : REGION_MILESTONES[reached - 1]!;
  const next = REGION_MILESTONES[reached]!;
  return { level: reached, nextPercent: next, progress: Math.min(1, Math.max(0, (percent - previous) / (next - previous))) };
}

/**
 * A percentage people can read at any size: "12.3%", "1.25%", "0.024%",
 * "0.00013%". Small shares keep two significant digits instead of
 * rounding to zero, because the first steps are the ones worth showing.
 */
export function formatPercent(percent: number): string {
  if (!(percent > 0)) return "0%";
  if (percent >= 99.95) return "100%";
  if (percent >= 10) return `${percent.toFixed(1)}%`;
  if (percent >= 1) return `${percent.toFixed(2)}%`;
  const decimals = Math.min(9, Math.ceil(-Math.log10(percent)) + 1);
  return `${Number(percent.toFixed(decimals))
    .toFixed(decimals)
    .replace(/0+$/, "")
    .replace(/\.$/, "")}%`;
}

/** What was last celebrated, so a level-up is announced once. */
export interface SeenLevels {
  explorer: number;
  regions: Record<string, number>;
}

export interface LevelUps {
  explorer?: number;
  /** Region IDs whose level went up, with the new level. */
  regions: { id: string; level: number }[];
  seen: SeenLevels;
}

/**
 * Compares the current levels with the ones seen last time. A level that
 * was never seen before is recorded without fanfare: opening the page on a
 * new device must not replay every level-up of the past.
 */
export function levelUps(seen: SeenLevels | null, explorer: number, regions: { id: string; level: number }[]): LevelUps {
  const next: SeenLevels = { explorer, regions: { ...(seen?.regions ?? {}) } };
  const ups: LevelUps = { regions: [], seen: next };
  if (seen && explorer > seen.explorer) ups.explorer = explorer;
  for (const region of regions) {
    const before = seen?.regions[region.id];
    // A region met for the first time has its own welcome (the "new region" event), not a level-up.
    if (before !== undefined && region.level > before) ups.regions.push(region);
    next.regions[region.id] = region.level;
  }
  return ups;
}
