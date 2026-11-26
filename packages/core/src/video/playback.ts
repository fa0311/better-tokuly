/** What the reporter needs from the outside: the two RPCs and the player's state. */
export interface PlaybackReporterDeps<Source> {
  startPlayback(request: {
    videoId: string;
    source: Source;
    positionSeconds: number;
  }): Promise<{ sessionId: string; reportIntervalSeconds: number; viewCount: bigint }>;
  reportPlayback(request: {
    sessionId: string;
    positionSeconds: number;
    playing: boolean;
    ended: boolean;
  }): Promise<{ viewCount: bigint }>;
  /** Where playback is, in seconds. */
  position(): number;
  /** The video is moving right now (not paused, stalled or hidden). */
  isPlaying(): boolean;
  /** The error says the session expired or belongs to a video that is gone. */
  isSessionGone(error: unknown): boolean;
  onViewCount?(count: bigint): void;
}

/**
 * One viewing of one video, reported the way the API asks: a session is
 * opened when playback first starts, then the position is sent at the
 * server's cadence and whenever `report` is called (pause, seek, end).
 *
 * `dispose` ends it for good — also for a session whose opening is still
 * on its way: a late answer is dropped, and no timer outlives the viewing.
 */
export class PlaybackReporter<Source> {
  private session: string | null = null;
  private starting = false;
  private disposed = false;
  private timer: ReturnType<typeof setInterval> | undefined;

  constructor(
    private readonly videoId: string,
    private readonly source: Source,
    private readonly deps: PlaybackReporterDeps<Source>,
  ) {}

  /** The open session, for a last report sent by other means as the page goes away. */
  get sessionId(): string | null {
    return this.session;
  }

  /** Playback started or resumed: opens the session, or reports into the open one. */
  play(): void {
    if (this.session) void this.report();
    else void this.start();
  }

  async report(ended = false): Promise<void> {
    const session = this.session;
    if (!session || this.disposed) return;
    try {
      const response = await this.deps.reportPlayback({
        sessionId: session,
        positionSeconds: this.deps.position(),
        playing: !ended && this.deps.isPlaying(),
        ended,
      });
      if (!this.disposed) this.deps.onViewCount?.(response.viewCount);
    } catch (error) {
      // The next play, or the next tick that finds the video playing, opens a new session.
      if (this.deps.isSessionGone(error) && this.session === session) this.session = null;
    }
  }

  dispose(): void {
    this.disposed = true;
    this.session = null;
    clearInterval(this.timer);
    this.timer = undefined;
  }

  private async start(): Promise<void> {
    if (this.disposed || this.session || this.starting) return;
    this.starting = true;
    try {
      const response = await this.deps.startPlayback({
        videoId: this.videoId,
        source: this.source,
        positionSeconds: this.deps.position(),
      });
      // The viewer may have left while the request was on its way.
      if (this.disposed) return;
      this.session = response.sessionId;
      this.deps.onViewCount?.(response.viewCount);
      clearInterval(this.timer);
      this.timer = setInterval(
        () => {
          if (this.session) void this.report();
          else if (this.deps.isPlaying()) void this.start();
        },
        Math.max(2, response.reportIntervalSeconds || 10) * 1000,
      );
    } catch {
      // Not ready, age-restricted or rate-limited: watching goes on unreported.
    } finally {
      this.starting = false;
    }
  }
}
