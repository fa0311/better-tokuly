/** How quest's celebrations queue and how loud they are: no schema needed, so the reveal layer can load early. */

/**
 * How loud a celebration is, from the ドパガキ度 factor (0 with motion off):
 * - `toast`: words only;
 * - `card`: a card pops in over the page, with a little confetti;
 * - `full`: the whole screen lights up.
 */
export type Loudness = "toast" | "card" | "full";

export function loudness(factor: number): Loudness {
  if (factor >= 1.3) return "full";
  if (factor >= 0.75) return "card";
  return "toast";
}

/**
 * Adds a reveal at the end. The first one is on screen and always stays;
 * past the limit the oldest of those still waiting make room.
 */
export function enqueue<T>(queue: T[], item: T, limit: number): T[] {
  const next = [...queue, item];
  if (next.length <= limit) return next;
  const [shown, ...waiting] = next;
  return shown === undefined ? next : [shown, ...waiting.slice(-(limit - 1))];
}
