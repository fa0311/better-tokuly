import {
  type QueryClient,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { notes } from "@tokuly/api";
import { api } from "#/lib/api";

export const channelClient = () => api().service(notes.NoteChannelService);

/** The three lists of the channel directory. */
export type ChannelTab = "featured" | "followed" | "managed";

const FILTERS: Record<ChannelTab, notes.ChannelFilter> = {
  featured: notes.ChannelFilter.FEATURED,
  followed: notes.ChannelFilter.FOLLOWED,
  managed: notes.ChannelFilter.MANAGED,
};

/** Colours a channel can be given; empty is the app's default. Stored as `#rrggbb`. */
export const CHANNEL_COLORS = [
  "#ff5c8a",
  "#ff8a3d",
  "#ffc83d",
  "#7bd94a",
  "#2ec4b6",
  "#4d9dff",
  "#8b6cff",
  "#c86bfa",
] as const;

export const CHANNEL_NAME_MAX = 50;
export const CHANNEL_DESCRIPTION_MAX = 500;

/**
 * Query keys of channels. They live under `["notes"]`, so the live updates of
 * notes (counters, deletions) reach the channel timelines too.
 */
export const channelKeys = {
  all: ["notes", "channels"] as const,
  lists: ["notes", "channels", "list"] as const,
  list: (tab: ChannelTab, query: string) => ["notes", "channels", "list", tab, query] as const,
  channel: (channelId: string) => ["notes", "channels", "one", channelId] as const,
  timeline: (channelId: string) => ["notes", "channels", "timeline", channelId] as const,
  moderators: (channelId: string) => ["notes", "channels", "moderators", channelId] as const,
};

export function useChannels(tab: ChannelTab, query: string, enabled = true) {
  return useInfiniteQuery({
    queryKey: channelKeys.list(tab, query),
    queryFn: ({ pageParam }) =>
      channelClient().listChannels({
        filter: FILTERS[tab],
        query,
        page: { cursor: pageParam, limit: 30 },
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled,
  });
}

export function useChannel(channelId: string) {
  return useQuery({
    queryKey: channelKeys.channel(channelId),
    queryFn: async () => (await channelClient().getChannel({ channelId })).channel!,
  });
}

/** One page of a channel's notes; the pinned notes come with the first. */
export interface ChannelTimelinePage {
  entries: notes.TimelineEntry[];
  pinned: notes.Note[];
  nextCursor: string;
}

export function useChannelTimeline(channelId: string) {
  return useInfiniteQuery({
    queryKey: channelKeys.timeline(channelId),
    queryFn: async ({ pageParam }): Promise<ChannelTimelinePage> => {
      const response = await channelClient().getChannelTimeline({
        channelId,
        page: { cursor: pageParam, limit: 30 },
      });
      return {
        entries: response.entries,
        pinned: response.pinnedNotes,
        nextCursor: response.page?.nextCursor ?? "",
      };
    },
    initialPageParam: "",
    getNextPageParam: (last) => last.nextCursor || undefined,
  });
}

export function useChannelModerators(channelId: string, enabled = true) {
  return useQuery({
    queryKey: channelKeys.moderators(channelId),
    queryFn: async () => (await channelClient().listChannelModerators({ channelId })).moderators,
    enabled,
  });
}

/** Puts a channel as the server returned it into the cache, and lets the lists load again. */
export function storeChannel(queryClient: QueryClient, channel: notes.NoteChannel): void {
  queryClient.setQueryData(channelKeys.channel(channel.id), channel);
  void queryClient.invalidateQueries({ queryKey: channelKeys.lists });
}

/** After a moderator action: the channel (pins, counts) and its notes load again. */
export function useRefreshChannel(channelId: string) {
  const queryClient = useQueryClient();
  return () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: channelKeys.channel(channelId) }),
      queryClient.invalidateQueries({ queryKey: channelKeys.timeline(channelId) }),
    ]);
}
