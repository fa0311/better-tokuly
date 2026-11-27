import { common, live } from "@tokuly/api";
import type { Subject } from "@tokuly/core/engagement";
import { activeHighlights, authorName, giftEntry, mentionOf, pollVisible } from "@tokuly/core/live";
import { Pin, X } from "lucide-react-native";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, FlatList, Pressable, ScrollView, Text, View } from "react-native";
import { EmptyState, Spinner } from "#/components/page";
import { useMe } from "#/features/session/store";
import { errorMessage } from "#/lib/errors";
import { afterSheet, MessageActions, ReportSheet } from "./actions";
import { ChatLine, GiftBubble } from "./chat-line";
import { ChatComposer, type ComposerHandle } from "./composer";
import { chatClient, useBlockedIds, useGiftTiers } from "./data";
import { GiftSheet } from "./gift-sheet";
import { useNow } from "./parts";
import { PollCard } from "./poll-card";
import { useChatMessages } from "./room";

const { StreamStatus } = live;

/** The message a streamer or moderator pinned to the top of the chat. */
function PinnedMessage({ stream }: { stream: live.Stream }) {
  const { t } = useTranslation("live");
  // A pinned line of someone the viewer blocked stays hidden (also when the stream was read with it).
  const blocked = useBlockedIds().data;
  const pinnedBy = stream.pinnedMessage?.author?.user?.id ?? "";
  const pinned = pinnedBy && blocked?.has(pinnedBy) ? undefined : stream.pinnedMessage;
  if (!pinned) return null;
  const unpin = async () => {
    const client = chatClient();
    try {
      await client.unpinChatMessage({ streamId: stream.summary?.id ?? "" });
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "live" }));
    }
  };
  return (
    <View
      testID="pinned-message"
      className="mx-3 mt-2 flex-row items-start gap-2 rounded-2xl bg-brand-soft px-3 py-2"
    >
      <Pin size={16} color="#5f3df0" accessibilityLabel={t("chat.pinnedLabel")} />
      <Text className="min-w-0 flex-1 text-sm text-ink">
        <Text className="font-black text-brand-strong">{authorName(pinned)} </Text>
        {pinned.text}
      </Text>
      {stream.viewer?.isModerator && (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("chat.unpin")}
          onPress={() => void unpin()}
          className="p-1"
        >
          <X size={16} color="#5d5778" />
        </Pressable>
      )}
    </View>
  );
}

/** The gifts that are still highlighted, as chips above the chat; a tap shows the whole gift. */
function Highlights({ stream, tiers }: { stream: live.Stream; tiers: live.GiftTier[] }) {
  const { t } = useTranslation("live");
  const now = useNow(5000);
  const [open, setOpen] = useState<string | null>(null);
  const gifts = activeHighlights(stream.highlightedGifts, now);
  if (gifts.length === 0) return null;
  const shown = gifts.find((gift) => gift.id === open);
  return (
    <View testID="gift-highlights" className="mt-2 gap-2">
      <ScrollView horizontal showsHorizontalScrollIndicator={false}>
        <View className="flex-row gap-1.5 px-3">
          {gifts.map((gift) => (
            <Pressable
              key={gift.id}
              accessibilityRole="button"
              accessibilityLabel={t("gift.highlightBy", { name: authorName(gift) })}
              accessibilityState={{ expanded: open === gift.id }}
              onPress={() => setOpen(open === gift.id ? null : gift.id)}
            >
              <GiftBubble message={gift} tiers={tiers} compact />
            </Pressable>
          ))}
        </View>
      </ScrollView>
      {shown && (
        <View className="mx-3">
          <GiftBubble message={shown} tiers={tiers} />
        </View>
      )}
    </View>
  );
}

/**
 * The live chat of a stream: the pinned message, highlighted gifts and the
 * open poll on top, the lines (newest at the bottom), and the composer
 * with the gift button. A long press on a line opens what can be done with
 * it.
 */
export function LiveChat({ stream }: { stream: live.Stream }) {
  const { t } = useTranslation("live");
  const me = useMe();
  const streamId = stream.summary?.id ?? "";
  const status = stream.summary?.status;
  const chat = useChatMessages(streamId);
  const tiers = useGiftTiers().data?.tiers ?? [];
  const now = useNow(1000);
  const composer = useRef<ComposerHandle>(null);
  const [selected, setSelected] = useState<live.ChatMessage | null>(null);
  const [reporting, setReporting] = useState<Subject | null>(null);
  const [gifting, setGifting] = useState(false);

  const viewer = stream.viewer;
  const writable = status === StreamStatus.UPCOMING || status === StreamStatus.LIVE;
  const gifts = giftEntry(stream);
  const canGift = gifts === "send";
  const poll = pollVisible(stream.activePoll, now) ? stream.activePoll : undefined;
  const selfHandle = me?.user?.handle.toLowerCase();
  const mention = (message: live.ChatMessage) => composer.current?.insert(mentionOf(message));
  // Newest first for the inverted list, so the newest line sits on the composer.
  const lines = [...(chat.data ?? [])].reverse();

  return (
    <View testID="live-chat" className="min-h-0 flex-1 bg-surface">
      <PinnedMessage stream={stream} />
      <Highlights stream={stream} tiers={tiers} />
      {poll && (
        <PollCard
          poll={poll}
          canVote={Boolean(viewer) && writable}
          canManage={Boolean(viewer?.isModerator)}
        />
      )}
      {chat.isPending ? (
        <View className="flex-1">
          <Spinner />
        </View>
      ) : lines.length === 0 ? (
        <View className="flex-1 justify-center">
          <EmptyState
            emoji="💬"
            title={t(writable ? "chat.emptyTitle" : "chat.emptyEndedTitle")}
            body={writable ? t("chat.emptyBody") : undefined}
          />
        </View>
      ) : (
        <FlatList
          testID="live-chat-lines"
          inverted
          data={lines}
          keyExtractor={(message) => message.id}
          keyboardShouldPersistTaps="handled"
          contentContainerClassName="py-2"
          renderItem={({ item }) => (
            <ChatLine
              message={item}
              tiers={tiers}
              selfHandle={selfHandle}
              onMention={writable ? mention : undefined}
              onMore={setSelected}
            />
          )}
        />
      )}
      {gifts === "age-restricted" && (
        <Text
          testID="gifts-age-restricted"
          className="border-t-2 border-line px-4 py-2 text-sm font-bold text-ink-soft"
        >
          🎁 {t("gift.ageRestricted")}
        </Text>
      )}
      {writable && (
        <ChatComposer
          ref={composer}
          stream={stream}
          onGift={canGift ? () => setGifting(true) : undefined}
        />
      )}
      <MessageActions
        message={selected}
        stream={stream}
        selfId={me?.user?.id}
        onClose={() => setSelected(null)}
        onMention={mention}
        onReport={(message) =>
          afterSheet(() =>
            setReporting({ type: common.SubjectType.LIVE_CHAT_MESSAGE, id: message.id }),
          )
        }
      />
      <ReportSheet subject={reporting} onClose={() => setReporting(null)} />
      {canGift && <GiftSheet stream={stream} visible={gifting} onClose={() => setGifting(false)} />}
    </View>
  );
}
