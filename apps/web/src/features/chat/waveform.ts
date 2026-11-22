/** Waveforms of voice messages: the peaks the server stores, and the bars drawn from them. */

/** How many peaks a voice message carries (what the server keeps, too). */
export const PEAK_COUNT = 64;

/** `count` bar heights (0..1) from stored peaks (0-255). A missing waveform draws as a flat line. */
export function waveformBars(peaks: ArrayLike<number>, count = 40): number[] {
  if (peaks.length === 0) return Array.from({ length: count }, () => 0.12);
  const bars: number[] = [];
  for (let index = 0; index < count; index++) {
    const from = Math.floor((index * peaks.length) / count);
    const to = Math.max(from + 1, Math.floor(((index + 1) * peaks.length) / count));
    let peak = 0;
    for (let position = from; position < to; position++) peak = Math.max(peak, peaks[position] ?? 0);
    bars.push(Math.max(0.12, Math.min(1, peak / 255)));
  }
  return bars;
}

/**
 * Reduces the loudness samples taken while recording (0..1, any number of
 * them) to `PEAK_COUNT` peaks of 0-255, scaled so the loudest moment fills
 * the bar.
 */
export function peaksOf(levels: number[], count = PEAK_COUNT): Uint8Array {
  const peaks = new Uint8Array(count);
  if (levels.length === 0) return peaks;
  const loudest = Math.max(...levels, 0.01);
  for (let index = 0; index < count; index++) {
    const from = Math.floor((index * levels.length) / count);
    const to = Math.max(from + 1, Math.floor(((index + 1) * levels.length) / count));
    let peak = 0;
    for (let position = from; position < to && position < levels.length; position++) peak = Math.max(peak, levels[position]!);
    peaks[index] = Math.round(Math.min(1, peak / loudest) * 255);
  }
  return peaks;
}
