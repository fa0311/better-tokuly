import { useQuery, useQueryClient } from "@tanstack/react-query";
import { errorReason, live } from "@tokuly/api";
import { LiveChats, liveKeys, patchStream, refreshStream } from "@tokuly/core/live";
import { useEffect, useRef, useState } from "react";
import { realtime } from "#/lib/realtime";
import { deviceId } from "#/platform/device";
import { chatClient, streamClient, useBlockedIds } from "./data";

const { StreamStatus } = live;

/**
 * The chats of the streams open in the app (`LiveChats` of
 * `@tokuly/core/live`). Its client is `api()` at each read: a switch starts
 * the signed-in tree over, and with it every open chat.
 */
export const liveChats = new LiveChats(chatClient);

/** The chat of a stream, oldest first: the latest page, then whatever is pushed. */
export function useChatMessages(streamId: string) {
  return useQuery(liveChats.query(streamId));
}

/**
 * Keeps an open stream current: subscribes to the stream's topic and applies
 * what is pushed (numbers, chat, pins, polls, the end), and reads the stream
 * and its chat again whenever the subscription comes back from a gap.
 * Returns the raid the stream ended with, if it happened while watching.
 */
export function useLiveRoom(stream: live.Stream): live.Raid | null {
  const queryClient = useQueryClient();
  const streamId = stream.summary?.id ?? "";
  const topic = stream.realtimeTopic;
  const [raid, setRaid] = useState<live.Raid | null>(null);
  // Whom the viewer blocked, so the room can leave those lines out.
  useBlockedIds();

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
        liveChats.apply(queryClient, streamId, event);
        return;
      }
      if (frame.topic) return;
      // Sent to this person alone: their standing in the chat, their own stream.
      if (event.case === "chatRestrictionChanged") refreshStream(queryClient, streamId);
      else if (event.case === "streamUpdated" && event.value.id === streamId)
        liveChats.apply(queryClient, streamId, event);
    });
    const { release } = client.subscribe(topic, {
      // In effect (first, or back after a gap): what was read before may lack what was pushed in between.
      onSubscribed: () => {
        refreshStream(queryClient, streamId);
        void queryClient.invalidateQueries({
          queryKey: liveKeys.chat(streamId),
          refetchType: "all",
        });
      },
      // Blocked by the streamer, unfriended, narrowed to members, deleted: the screen reads the
      // stream again and lands where a stranger would.
      onRevoked: () => refreshStream(queryClient, streamId),
    });
    client.start();
    return () => {
      offFrame();
      release();
    };
  }, [queryClient, topic, streamId]);

  return raid;
}

/**
 * The viewer's heartbeat while a stream is on air and on screen: it keeps
 * them in the audience and brings fresh numbers and playback URLs (the token
 * of a restricted stream lasts a quarter of an hour). The client is taken
 * when watching starts, so a beat never goes out as another account.
 */
export function useWatchingReport(stream: live.Stream, enabled: boolean): void {
  const queryClient = useQueryClient();
  const streamId = stream.summary?.id ?? "";
  const onAir = stream.summary?.status === StreamStatus.LIVE;
  const interval = useRef(stream.reportIntervalSeconds || 15);

  useEffect(() => {
    if (!enabled || !onAir || !streamId) return;
    const client = streamClient();
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const report = async () => {
      try {
        const answer = await client.reportWatching({ streamId, clientId: await deviceId() });
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
        if (stopped) return;
        // Gone for this viewer (unfriended, blocked, made private, deleted): read the stream
        // again and land where a stranger would. Anything else is made up for next time.
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
