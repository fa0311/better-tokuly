import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import { errorReason, live } from "@tokuly/api";
import { pushesDuringReadByKey } from "@tokuly/core/query";
import { useEffect, useRef, useState } from "react";
import { deviceId, realtime } from "#/lib/realtime";
import {
  chatClient,
  liveKeys,
  patchStream,
  refreshStream,
  streamClient,
  useBlockedIds,
} from "./api";
import { addChatMessages, mergePoll, removeChatMessage, removeChatUser } from "./model";

const { ChatMessageKind, StreamStatus } = live;

/**
 * What pushes did to a stream's chat while it was being read (lines added,
 * deleted, cleared): the answer may predate them, so they are applied to it
 * again (see `pushesDuringRead`).
 */
const chatPushesDuringRead = pushesDuringReadByKey<live.ChatMessage[]>();

/** Reads the latest page of a stream's chat, with what was pushed while it was read. */
export async function readChat(streamId: string): Promise<live.ChatMessage[]> {
  const read = chatPushesDuringRead.begin(streamId);
  try {
    const { messages } = await chatClient().listChatMessages({ streamId });
    return read.finish(addChatMessages([], messages));
  } catch (error) {
    read.cancel();
    throw error;
  }
}

/** The chat of a stream, oldest first: the latest page, then whatever is pushed. */
export function useChatMessages(streamId: string) {
  return useQuery({
    queryKey: liveKeys.chat(streamId),
    queryFn: () => readChat(streamId),
    // Pushes keep it current; reading it again would only drop what the page did not hold.
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 0,
  });
}

function updateChat(
  queryClient: QueryClient,
  streamId: string,
  update: (messages: live.ChatMessage[]) => live.ChatMessage[],
): void {
  chatPushesDuringRead.record(streamId, update);
  queryClient.setQueryData<live.ChatMessage[]>(liveKeys.chat(streamId), (messages) =>
    messages ? update(messages) : messages,
  );
}

/** Adds a line the caller just sent (the push may come before or after the answer). */
export function addOwnMessage(queryClient: QueryClient, message: live.ChatMessage): void {
  updateChat(queryClient, message.streamId, (messages) => addChatMessages(messages, [message]));
  if (message.kind === ChatMessageKind.GIFT) addHighlight(queryClient, message);
}

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

/** Applies one pushed `LiveEvent` of a stream to what is loaded. */
export function applyLiveEvent(
  queryClient: QueryClient,
  streamId: string,
  event: live.LiveEvent["event"],
): void {
  // The server leaves out the viewer when they are on a line's exception list, but that list
  // may drop the people who blocked its author (it is capped, and anyone can block anyone).
  // So lines of people the viewer blocked stay out here, and so does such a pinned line.
  const blocked = queryClient.getQueryData<Set<string>>(liveKeys.blocked);
  const fromBlocked = (message: live.ChatMessage | undefined) => {
    const authorId = message?.author?.user?.id;
    return Boolean(authorId && blocked?.has(authorId));
  };
  if (event.case === "chatMessage" && fromBlocked(event.value)) return;
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
      updateChat(queryClient, streamId, (messages) => addChatMessages(messages, [message]));
      if (message.kind === ChatMessageKind.GIFT) addHighlight(queryClient, message);
      break;
    }
    case "chatMessageDeleted": {
      const { messageId } = event.value;
      updateChat(queryClient, streamId, (messages) => removeChatMessage(messages, messageId));
      patchStream(queryClient, streamId, (stream) => ({
        ...stream,
        highlightedGifts: stream.highlightedGifts.filter((gift) => gift.id !== messageId),
        pinnedMessage: stream.pinnedMessage?.id === messageId ? undefined : stream.pinnedMessage,
      }));
      break;
    }
    case "chatUserCleared": {
      const { userId } = event.value;
      updateChat(queryClient, streamId, (messages) => removeChatUser(messages, userId));
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

export interface LiveRoom {
  /** Set when the stream ended by sending its viewers to another one. */
  raid: live.Raid | null;
}

/**
 * Keeps an open stream page current: subscribes to the stream's topic and
 * applies what is pushed (numbers, chat, pins, polls, the end), and reads
 * the page again after a gap in the connection.
 */
export function useLiveRoom(stream: live.Stream): LiveRoom {
  const queryClient = useQueryClient();
  const streamId = stream.summary?.id ?? "";
  const topic = stream.realtimeTopic;
  const [raid, setRaid] = useState<live.Raid | null>(null);
  // Signed-in viewers: whom they blocked, so the room can leave those lines out.
  useBlockedIds(Boolean(stream.viewer));

  useEffect(() => {
    if (!topic || !streamId) return;
    const client = realtime();
    const offFrame = client.onFrame((frame) => {
      if (frame.body.case === "social") {
        // A block (or an ended friendship) on this account: read the block list again.
        if (frame.body.value.event.case === "friendRemovedUserId")
          void queryClient.invalidateQueries({ queryKey: liveKeys.blocked });
        return;
      }
      if (frame.body.case !== "live") return;
      const event = frame.body.value.event;
      if (frame.topic === topic) {
        if (event.case === "raid") setRaid(event.value);
        applyLiveEvent(queryClient, streamId, event);
        return;
      }
      if (frame.topic) return;
      // Sent to this person alone: their standing in the chat, their encoder.
      if (event.case === "chatRestrictionChanged") {
        refreshStream(queryClient, streamId);
      } else if (event.case === "encoderState" && event.value.streamId === streamId) {
        const { connected, protocol } = event.value;
        patchStream(queryClient, streamId, (current) => ({
          ...current,
          studio: current.studio && {
            ...current.studio,
            encoderConnected: connected,
            encoderProtocol: protocol,
          },
        }));
        refreshStream(queryClient, streamId);
      } else if (event.case === "streamUpdated" && event.value.id === streamId) {
        applyLiveEvent(queryClient, streamId, event);
      }
    });
    const { release } = client.subscribe(topic, {
      // In effect (first, or back after a gap): the stream and its chat as read before may lack
      // what was pushed in between (the user's own events too, which a reconnect also missed).
      onSubscribed: () => {
        refreshStream(queryClient, streamId);
        void queryClient.invalidateQueries({
          queryKey: liveKeys.chat(streamId),
          refetchType: "all",
        });
      },
      // Blocked by the streamer, unfriended, narrowed to members, deleted: the page reads the
      // stream again and lands where a stranger would (as when a watching report says so).
      onRevoked: () => refreshStream(queryClient, streamId),
    });
    client.start();
    return () => {
      offFrame();
      release();
    };
  }, [queryClient, topic, streamId]);

  return { raid };
}

/**
 * The viewer's heartbeat while a stream is on air: it keeps them in the
 * audience and brings fresh numbers and playback URLs (the token of a
 * restricted stream lasts a quarter of an hour).
 */
export function useWatchingReport(stream: live.Stream, enabled: boolean): void {
  const queryClient = useQueryClient();
  const streamId = stream.summary?.id ?? "";
  const onAir = stream.summary?.status === StreamStatus.LIVE;
  const interval = useRef(stream.reportIntervalSeconds || 15);

  useEffect(() => {
    if (!enabled || !onAir || !streamId) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const report = async () => {
      try {
        const answer = await streamClient().reportWatching({ streamId, clientId: deviceId() });
        if (stopped) return;
        interval.current = answer.reportIntervalSeconds || interval.current;
        patchStream(queryClient, streamId, (current) => ({
          ...current,
          // The next media session opens with these; the one that is playing is left alone.
          playback: answer.playback ?? current.playback,
          summary: current.summary && {
            ...current.summary,
            viewerCount: answer.viewerCount,
            likeCount: answer.likeCount,
          },
        }));
        if (answer.status !== StreamStatus.LIVE) refreshStream(queryClient, streamId);
      } catch (error) {
        // Gone for this viewer (unfriended, blocked, made private, deleted): the page reads the
        // stream again and lands where a stranger would. Anything else is made up for next time.
        if (errorReason(error) === "STREAM_NOT_FOUND") {
          refreshStream(queryClient, streamId);
          return;
        }
      }
      if (!stopped) timer = setTimeout(report, interval.current * 1000);
    };
    void report();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [queryClient, streamId, onAir, enabled]);
}
