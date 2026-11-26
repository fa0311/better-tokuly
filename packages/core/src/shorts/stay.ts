/** What one stay on one short amounts to. */
export interface Stay {
  watchedMs: number;
  completions: number;
}

/**
 * Measures one stay on one short: how long it really played (waiting for
 * data and pauses do not count, replays do) and how often it reached its end.
 * Fed by the media element's events; `now` is a monotonic clock in
 * milliseconds.
 */
export class StayMeter {
  private watched = 0;
  private completions = 0;
  private playingSince: number | null = null;

  /** Playback is moving. */
  play(now: number): void {
    this.playingSince ??= now;
  }

  /** Playback stopped: paused, stalled or off screen. */
  pause(now: number): void {
    if (this.playingSince === null) return;
    this.watched += Math.max(0, now - this.playingSince);
    this.playingSince = null;
  }

  /** Playback reached the end of the short (it loops from there). */
  complete(now: number): void {
    this.pause(now);
    this.completions += 1;
  }

  /** The stay so far. */
  read(now: number): Stay {
    const running = this.playingSince === null ? 0 : Math.max(0, now - this.playingSince);
    return { watchedMs: Math.round(this.watched + running), completions: this.completions };
  }
}
