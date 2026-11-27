/**
 * Footprint logic every app shares: query keys and options, region names,
 * and the game around the scratch map (explorer and region levels, readable
 * percentages, which level-ups are new). Drawing the fog stays in each app.
 */
export { footprintKeys, regionName } from "./keys";
export {
  cellsForLevel,
  type ExplorerLevel,
  explorerLevel,
  formatPercent,
  type LevelUps,
  levelUps,
  REGION_MILESTONES,
  type RegionLevel,
  regionLevel,
  type SeenLevels,
} from "./levels";
export { footprintQueries } from "./queries";
