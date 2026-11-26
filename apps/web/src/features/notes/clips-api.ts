import { type QueryClient, useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { notes } from "@tokuly/api";
import { clipKeys } from "@tokuly/core/notes";
import { api } from "#/lib/api";

export const clipClient = () => api().service(notes.ClipService);

export const CLIP_NAME_MAX = 50;
export const CLIP_DESCRIPTION_MAX = 500;

export function useClips(userId: string, noteId = "", enabled = true) {
  return useInfiniteQuery({
    queryKey: clipKeys.list(userId, noteId),
    queryFn: ({ pageParam }) =>
      clipClient().listClips({ userId, noteId, page: { cursor: pageParam, limit: 50 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled,
  });
}

export function useClip(clipId: string) {
  return useQuery({
    queryKey: clipKeys.clip(clipId),
    queryFn: async () => (await clipClient().getClip({ clipId })).clip!,
  });
}

export function useClipNotes(clipId: string) {
  return useInfiniteQuery({
    queryKey: clipKeys.notes(clipId),
    queryFn: ({ pageParam }) =>
      clipClient().getClipNotes({ clipId, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

/** A clip changed: store it and let the lists and its notes load again. */
export function storeClip(queryClient: QueryClient, clip: notes.Clip): void {
  queryClient.setQueryData(clipKeys.clip(clip.id), clip);
  void queryClient.invalidateQueries({ queryKey: clipKeys.lists });
  void queryClient.invalidateQueries({ queryKey: clipKeys.notes(clip.id) });
}
