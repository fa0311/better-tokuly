import { media } from "@tokuly/api";
import { useEffect, useRef, useState } from "react";
import { pickFromLibrary } from "#/features/media/pick";
import { uploadFile } from "#/features/media/upload";
import { type Account, currentAccount, isActive } from "#/features/session/account";
import { forgetFiles, keepFile } from "#/platform/transfer";

/** The most photos a note takes (the server's limit). */
export const MAX_PHOTOS = 10;

export interface NotePhoto {
  key: string;
  /** What the tile shows: the picked file, or a draft's stored image. */
  uri: string;
  /** Set once the upload is processed (or for a draft's photo, from the start). */
  assetId?: string;
  progress: number;
  error?: unknown;
}

let counter = 0;

/**
 * The photos of a note being written. Each goes up as soon as it is picked,
 * as the account that opened the library (captured before the library is
 * awaited: pictures chosen after another account took over, or after the
 * composer closed, are dropped; the upload stops when that account is no
 * longer the active one), kept in the app's documents while it goes. Leaving
 * the composer stops what is still going and throws the copies away; the
 * server drops unused uploads itself.
 */
export function useNotePhotos(initial: NotePhoto[] = []) {
  const [photos, setPhotos] = useState<NotePhoto[]>(initial);
  const aborts = useRef(new Map<string, AbortController>());
  const closed = useRef(false);
  useEffect(() => {
    const running = aborts.current;
    closed.current = false;
    return () => {
      closed.current = true;
      for (const [key, abort] of running) {
        abort.abort();
        forgetFiles(key);
      }
    };
  }, []);

  const patch = (key: string, change: Partial<NotePhoto>) =>
    setPhotos((current) =>
      current.map((photo) => (photo.key === key ? { ...photo, ...change } : photo)),
    );

  const upload = async (
    account: Account,
    key: string,
    picked: { uri: string; name: string; contentType: string },
  ) => {
    const abort = new AbortController();
    aborts.current.set(key, abort);
    try {
      const kept = await keepFile(key, picked.uri, picked.name);
      const asset = await uploadFile(
        account,
        media.AssetPurpose.NOTE_IMAGE,
        { uri: kept.uri, name: picked.name, contentType: picked.contentType, size: kept.size },
        {
          until: "ready",
          save: () => {},
          onProgress: (progress) => patch(key, { progress }),
          signal: abort.signal,
        },
      );
      patch(key, { assetId: asset.id, progress: 1 });
    } catch (error) {
      if (!abort.signal.aborted) patch(key, { error });
    } finally {
      aborts.current.delete(key);
      forgetFiles(key);
    }
  };

  /** Picks photos from the library; `"denied"` when the library may not be read. */
  const pick = async (): Promise<"denied" | undefined> => {
    const account = currentAccount();
    const room = MAX_PHOTOS - photos.length;
    if (!account || room <= 0) return;
    const result = await pickFromLibrary({ kinds: ["images"], limit: room });
    if (!result) return;
    if ("denied" in result) return "denied";
    // The library was open for a while: the pictures belong to whoever opened it, in the composer still open.
    if (closed.current || !isActive(account)) return;
    const added = result.media.map((picked) => ({
      picked,
      key: `note-photo-${Date.now()}-${counter++}`,
    }));
    setPhotos((current) => [
      ...current,
      ...added.map(({ picked, key }) => ({ key, uri: picked.uri, progress: 0 })),
    ]);
    for (const { picked, key } of added) void upload(account, key, picked);
  };

  const remove = (key: string) => {
    aborts.current.get(key)?.abort();
    setPhotos((current) => current.filter((photo) => photo.key !== key));
  };

  return {
    photos,
    pick,
    remove,
    busy: photos.some((photo) => !photo.assetId && !photo.error),
    failed: photos.some((photo) => photo.error !== undefined),
    assetIds: photos.flatMap((photo) => (photo.assetId ? [photo.assetId] : [])),
  };
}
