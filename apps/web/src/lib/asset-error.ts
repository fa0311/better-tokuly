/**
 * An upload the server refused once it looked at it: `reason` is the asset's
 * error (`MEDIA_TOO_LONG`, `CORRUPT_MEDIA`...), and `maxDurationSeconds` the
 * longest recording its purpose takes (0 when there is no such limit).
 * `message` is the reason too, so code that only reads `message` still works.
 */
export class AssetFailedError extends Error {
  readonly reason: string;
  readonly maxDurationSeconds: number;

  constructor(reason: string, maxDurationSeconds = 0) {
    super(reason);
    this.name = "AssetFailedError";
    this.reason = reason;
    this.maxDurationSeconds = maxDurationSeconds;
  }
}
