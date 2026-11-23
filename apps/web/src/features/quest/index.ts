/**
 * What the quest feature shares with other features: a person's level, title
 * and the streak with them, small enough to sit next to a name.
 *
 *   <LevelChip userId={user.id} />      // "Lv.12"; nothing while loading or when they hide their progress
 *   <TitleBadge userId={user.id} />     // the title (称号) they wear; nothing when they wear none
 *   <StreakFlame with={friend.id} />    // "🔥12" ("⌛12" when it is about to end); nothing without a running streak
 *
 * `LevelChip` and `TitleBadge` read one cached lookup per person (shared by
 * both, kept for five minutes), so they suit headers and short lists, not a
 * feed of hundreds of different people. `StreakFlame` reads the signed-in
 * person's streak list once for every flame on screen.
 */
export { LevelChip, StreakFlame, TitleBadge } from "./chips";
