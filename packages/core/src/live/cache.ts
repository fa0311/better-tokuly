import type { QueryClient } from "@tanstack/query-core";
import { errorReason, live, type social } from "@tokuly/api";
import { type ClientSource, pushesDuringReadByKey } from "../query";
import { liveKeys } from "./keys";
import { addChatMessages, mergePoll, removeChatMessage, removeChatUser } from "./model";

const { ChatMessageKind } = live;

/**
 * Live streams in the query cache: a stream page under whichever ID it was
 * opened with (the UUID or the short ID), its chat, and what a pushed
 * `LiveEvent` does to both.
 */

/** Query options of the live domain. Each app adds `enabled` and its hooks around them. */
export const liveQueries = {
  /**
   * A stream page as the caller sees it; `null` when it does not exist (or is
   * not theirs to see). `streamId` is the UUID or the short ID.
   */
  stream: (client: ClientSource<typeof live.LiveStreamService>, streamId: string) => ({
    queryKey: liveKeys.stream(streamId),
    queryFn: async (): Promise<live.Stream | null> => {
      try {
        return (await client().getStream({ streamId })).stream ?? null;
      } catch (error) {
        if (errorReason(error) === "STREAM_NOT_FOUND") return null;
        throw error;
      }
    },
  }),

  /** The gift ladder. It never changes while the app is open. */
  tiers: (client: ClientSource<typeof live.LiveGiftService>) => ({
    queryKey: liveKeys.tiers,
    queryFn: () => client().getGiftTiers({}),
    staleTime: Number.POSITIVE_INFINITY,
  }),

  /**
   * The IDs of the people the signed-in viewer blocked. Chat history comes
   * filtered from the server, and so do pushed lines; a room still drops
   * pushed lines from these authors, as a second line of defence.
   */
  blocked: (client: ClientSource<typeof social.FriendService>) => ({
    queryKey: liveKeys.blocked,
    queryFn: async (): Promise<Set<string>> => {
      const ids = new Set<string>();
      let cursor = "";
      // A handful of pages is plenty; a viewer with more blocks than that still gets the newest ones.
      for (let page = 0; page < 5; page++) {
        const response = await client().listBlockedUsers({ page: { cursor, limit: 100 } });
        for (const user of response.users) ids.add(user.id);
        cursor = response.page?.nextCursor ?? "";
        if (!cursor) break;
      }
      return ids;
    },
    staleTime: 5 * 60_000,
  }),
};

/** Changes the cached page of a stream, under whichever ID it was opened with. */
export function patchStream(
  queryClient: QueryClient,
  streamId: string,
  patch: (stream: live.Stream) => live.Stream,
): void {
  for (const [key, stream] of queryClient.getQueriesData<live.Stream | null>({
    queryKey: liveKeys.streams,
  })) {
    if (!stream?.summary) continue;
    if (stream.summary.id !== streamId && stream.summary.shortId !== streamId) continue;
    queryClient.setQueryData(key, patch(stream));
  }
}

/** Puts an answer of the server (create, update, start, end) into the cache under both IDs. */
export function storeStream(queryClient: QueryClient, stream: live.Stream | undefined): void {
  if (!stream?.summary) return;
  queryClient.setQueryData(liveKeys.stream(stream.summary.id), stream);
  queryClient.setQueryData(liveKeys.stream(stream.summary.shortId), stream);
  void queryClient.invalidateQueries({ queryKey: liveKeys.lists });
}

/** Reads a stream's page again, under whichever ID it was opened with. */
export function refreshStream(queryClient: QueryClient, streamId: string): void {
  void queryClient.invalidateQueries({
    queryKey: liveKeys.streams,
    predicate: (query) => {
      const stream = query.state.data as live.Stream | null | undefined;
      return (
        query.queryKey[2] === streamId ||
        stream?.summary?.id === streamId ||
        stream?.summary?.shortId === streamId
      );
    },
  });
}

/**
 * The chats of the streams an app has open, oldest line first, in the cache
 * under `liveKeys.chat`. A chat is read once and then kept current by pushes;
 * after a gap in the connection it is read again, and what pushes did while
 * that read was on its way (lines added, deleted, cleared) is applied to the
 * answer again (see `pushesDuringRead`). One per app.
 */
export class LiveChats {
  private readonly duringRead = pushesDuringReadByKey<live.ChatMessage[]>();

  constructor(private readonly client: ClientSource<typeof live.LiveChatService>) {}

  /** Query options of a stream's chat: the latest page, then whatever is pushed. */
  query(streamId: string) {
    return {
      queryKey: liveKeys.chat(streamId),
      queryFn: () => this.read(streamId),
      // Pushes keep it current; reading it again would only drop what the page did not hold.
      staleTime: Number.POSITIVE_INFINITY,
      gcTime: 0,
    };
  }

  /** Reads the latest page of a stream's chat, with what was pushed while it was read. */
  async read(streamId: string): Promise<live.ChatMessage[]> {
    const read = this.duringRead.begin(streamId);
    try {
      const { messages } = await this.client().listChatMessages({ streamId });
      return read.finish(addChatMessages([], messages));
    } catch (error) {
      read.cancel();
      throw error;
    }
  }

  /** Changes a stream's cached chat, and every read of it in flight. */
  update(
    queryClient: QueryClient,
    streamId: string,
    update: (messages: live.ChatMessage[]) => live.ChatMessage[],
  ): void {
    this.duringRead.record(streamId, update);
    queryClient.setQueryData<live.ChatMessage[]>(liveKeys.chat(streamId), (messages) =>
      messages ? update(messages) : messages,
    );
  }

  /** Adds a line the caller just sent (the push may come before or after the answer). */
  addOwn(queryClient: QueryClient, message: live.ChatMessage): void {
    this.update(queryClient, message.streamId, (messages) => addChatMessages(messages, [message]));
    if (message.kind === ChatMessageKind.GIFT) addHighlight(queryClient, message);
  }

  /** Applies one pushed `LiveEvent` of a stream to what is loaded. */
  apply(queryClient: QueryClient, streamId: string, event: live.LiveEvent["event"]): void {
    // The server leaves out the viewer when they are on a line's exception list, but that list
    // may drop the people who blocked its author (it is capped, and anyone can block anyone).
    // So lines of people the viewer blocked stay out here, and so does such a pinned line.
    const blocked = queryClient.getQueryData<Set<string>>(liveKeys.blocked);
    const fromBlocked = (message: live.ChatMessage | undefined) => {
      const authorId = message?.author?.user?.id;
      return Boolean(authorId && blocked?.has(authorId));
    };
    switch (event.case) {
      case "streamUpdated": {
        const summary = event.value;
        patchStream(queryClient, streamId, (stream) => ({ ...stream, summary }));
        // Playback URLs, the archive and what the viewer may do come with the full page.
        refreshStream(queryClient, summary.id);
        void queryClient.invalidateQueries({ queryKey: liveKeys.lists });
        break;
      }
      case "stats": {
        const { viewerCount, peakViewerCount, likeCount } = event.value;
        patchStream(queryClient, streamId, (stream) => ({
          ...stream,
          summary: stream.summary && { ...stream.summary, viewerCount, peakViewerCount, likeCount },
        }));
        break;
      }
      case "chatMessage": {
        const message = event.value;
        if (fromBlocked(message)) break;
        this.update(queryClient, streamId, (messages) => addChatMessages(messages, [message]));
        if (message.kind === ChatMessageKind.GIFT) addHighlight(queryClient, message);
        break;
      }
      case "chatMessageDeleted": {
        const { messageId } = event.value;
        this.update(queryClient, streamId, (messages) => removeChatMessage(messages, messageId));
        patchStream(queryClient, streamId, (stream) => ({
          ...stream,
          highlightedGifts: stream.highlightedGifts.filter((gift) => gift.id !== messageId),
          pinnedMessage: stream.pinnedMessage?.id === messageId ? undefined : stream.pinnedMessage,
        }));
        break;
      }
      case "chatUserCleared": {
        const { userId } = event.value;
        this.update(queryClient, streamId, (messages) => removeChatUser(messages, userId));
        patchStream(queryClient, streamId, (stream) => ({
          ...stream,
          highlightedGifts: removeChatUser(stream.highlightedGifts, userId),
        }));
        break;
      }
      case "chatPinChanged": {
        const pinnedMessage = fromBlocked(event.value.message) ? undefined : event.value.message;
        patchStream(queryClient, streamId, (stream) => ({ ...stream, pinnedMessage }));
        break;
      }
      case "chatSettingsChanged": {
        const chat = event.value.chat;
        patchStream(queryClient, streamId, (stream) => ({ ...stream, chat }));
        // Whether the viewer may still write follows the new rules.
        refreshStream(queryClient, streamId);
        break;
      }
      case "pollUpdated": {
        const pushed = event.value;
        patchStream(queryClient, streamId, (stream) => ({
          ...stream,
          activePoll: mergePoll(stream.activePoll, pushed),
        }));
        void queryClient.invalidateQueries({ queryKey: liveKeys.polls(streamId) });
        break;
      }
      default:
        break;
    }
  }
}

/** Puts a gift line on top of the stream's highlights. */
function addHighlight(queryClient: QueryClient, message: live.ChatMessage): void {
  if (!message.gift?.highlightedUntil) return;
  patchStream(queryClient, message.streamId, (stream) => ({
    ...stream,
    highlightedGifts: [
      message,
      ...stream.highlightedGifts.filter((existing) => existing.id !== message.id),
    ],
  }));
}

/** A poll the server answered with (a vote, an end) in the stream's page. */
export function storePoll(queryClient: QueryClient, poll: live.Poll | undefined): void {
  if (!poll) return;
  patchStream(queryClient, poll.streamId, (stream) => ({
    ...stream,
    activePoll: mergePoll(stream.activePoll, poll),
  }));
  void queryClient.invalidateQueries({ queryKey: liveKeys.polls(poll.streamId) });
}
