import { useQueryClient } from "@tanstack/react-query";
import { type live, toMillis } from "@tokuly/api";
import { compactCount } from "@tokuly/core/engagement";
import { pollShares, storePoll } from "@tokuly/core/live";
import { BarChart3, Check } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { errorMessage } from "#/lib/errors";
import { pollClient } from "./data";
import { clock, useNow } from "./parts";

/**
 * A poll in a stream: tap an option to cast the one vote, then watch the
 * bars move as everyone's votes come in. The streamer and moderators can
 * close it early (polls are started from the web's control room).
 */
export function PollCard({
  poll,
  canVote,
  canManage,
}: {
  poll: live.Poll;
  canVote: boolean;
  canManage: boolean;
}) {
  const { t, i18n } = useTranslation("live");
  const queryClient = useQueryClient();
  const now = useNow();
  const [busy, setBusy] = useState(false);
  const closed = poll.endedAt !== undefined;
  const left = poll.endsAt ? Math.max(0, Math.ceil((toMillis(poll.endsAt) - now) / 1000)) : 0;
  const shares = pollShares(poll);
  const voted = poll.viewerChoice > 0;
  const showResults = voted || closed || !canVote;
  const leading = Math.max(...poll.options.map((option) => Number(option.voteCount)));

  const run = async (work: () => Promise<{ poll?: live.Poll }>) => {
    setBusy(true);
    try {
      storePoll(queryClient, (await work()).poll);
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "live" }));
    } finally {
      setBusy(false);
    }
  };
  const vote = (option: number) => {
    const client = pollClient();
    void run(() => client.votePoll({ pollId: poll.id, option }));
  };
  const end = () => {
    const client = pollClient();
    void run(() => client.endPoll({ pollId: poll.id }));
  };

  return (
    <View testID="poll-card" className="mx-3 mt-2 rounded-2xl border-2 border-line bg-surface p-3">
      <View className="flex-row items-start gap-2">
        <BarChart3 size={16} color="#7c5cff" />
        <Text className="min-w-0 flex-1 text-sm font-black text-ink">{poll.question}</Text>
        <Text className="text-xs font-bold text-ink-soft" style={{ fontVariant: ["tabular-nums"] }}>
          {closed ? t("poll.closed") : t("poll.left", { clock: clock(left) })}
        </Text>
      </View>
      <View className="mt-2 gap-1.5">
        {poll.options.map((option, index) => {
          const choice = index + 1;
          const mine = poll.viewerChoice === choice;
          const wins = closed && leading > 0 && Number(option.voteCount) === leading;
          return (
            <Pressable
              key={choice}
              testID={`poll-option-${choice}`}
              accessibilityRole="button"
              accessibilityState={{ selected: mine }}
              disabled={!canVote || voted || closed || busy}
              onPress={() => vote(choice)}
              className="h-10 flex-row items-center gap-2 overflow-hidden rounded-xl border-2 border-line px-3"
            >
              <View
                className={`absolute inset-y-0 left-0 ${mine || wins ? "bg-lime" : "bg-brand-soft"}`}
                style={{ width: showResults ? `${shares[index] ?? 0}%` : "0%" }}
              />
              {mine && <Check size={16} color="#17122b" accessibilityLabel={t("poll.yourVote")} />}
              <Text numberOfLines={1} className="flex-1 text-sm font-bold text-ink">
                {option.text}
              </Text>
              {showResults && (
                <Text testID="poll-share" className="text-sm font-bold text-ink">
                  {shares[index]}%
                </Text>
              )}
            </Pressable>
          );
        })}
      </View>
      <View className="mt-2 flex-row items-center justify-between">
        <Text testID="poll-votes" className="text-xs font-bold text-ink-faint">
          {t("poll.votes", { formatted: compactCount(Number(poll.totalVotes), i18n.language) })}
        </Text>
        {canManage && !closed && (
          <Pressable accessibilityRole="button" disabled={busy} onPress={end} className="px-2 py-1">
            <Text className="text-sm font-black text-ink-soft">{t("poll.end")}</Text>
          </Pressable>
        )}
      </View>
    </View>
  );
}
