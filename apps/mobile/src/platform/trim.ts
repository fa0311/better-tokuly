import { requireNativeModule } from "expo";

/** The local `tokuly-trim` module (`modules/tokuly-trim`): Media3 Transformer on Android, AVAssetExportSession on iOS. */
interface TrimModule {
  trim(uri: string, startMs: number, endMs: number): Promise<string>;
}

let native: TrimModule | null = null;

/**
 * Cuts the video at `uri` down to `[startMs, endMs)`; resolves to the URI of
 * the cut copy (an MP4 in the cache directory).
 */
export function trimFile(uri: string, startMs: number, endMs: number): Promise<string> {
  native ??= requireNativeModule<TrimModule>("TokulyTrim");
  return native.trim(uri, Math.round(startMs), Math.round(endMs));
}
