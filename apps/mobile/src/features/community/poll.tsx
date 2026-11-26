import { useQueryClient } from "@tanstack/react-query";
import { type community, toMillis } from "@tokuly/api";
import { patchThread, votePercent } from "@tokuly/core/community";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { errorMessage } from "#/lib/errors";
import { boardClient, NS } from "./data";

/**
 * A thread's poll: choose and vote while it is open; the results once the
 * server shows them (after voting, once closed, to the author).
 */
export function PollCard({ thread, canVote }: { thread: community.Thread; canVote: boolean }) {
  const { t, i18n } = useTranslation(NS);
  const queryClient = useQueryClient();
  const poll = thread.poll;
  const [chosen, setChosen] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  if (!poll) return null;
  const closes = poll.closesAt ? toMillis(poll.closesAt) : 0;
  const open = !poll.closed && (closes === 0 || closes > Date.now());
  const voted = poll.myOptionIds.length > 0;
  const choosing = open && canVote && !voted;

  const vote = async (optionIds: string[]) => {
    setBusy(true);
    try {
      const { poll: next } = await boardClient().votePoll({ threadId: thread.id, optionIds });
      if (next) patchThread(queryClient, thread.id, (each) => ({ ...each, poll: next }));
      setChosen([]);
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: NS }));
    } finally {
      setBusy(false);
    }
  };

  const toggle = (id: string) =>
    setChosen((current) =>
      poll.multipleChoice
        ? current.includes(id)
          ? current.filter((each) => each !== id)
          : [...current, id]
        : [id],
    );

  return (
    <View testID="thread-poll" className="gap-2 rounded-card bg-surface-2 p-3">
      <View className="flex-row items-center justify-between">
        <Text className="font-black text-ink">{`📊 ${t("poll.title")}`}</Text>
        <Text className="text-xs font-bold text-ink-soft">
          {open
            ? closes
              ? t("poll.closesAt", { time: new Date(closes).toLocaleString(i18n.language) })
              : t("poll.open")
            : t("poll.closed")}
        </Text>
      </View>
      {poll.options.map((option) => {
        const mine = poll.myOptionIds.includes(option.id);
        const picked = chosen.includes(option.id);
        const percent = votePercent(option, poll);
        return (
          <Pressable
            key={option.id}
            testID={`poll-option-${option.id}`}
            accessibilityRole={poll.multipleChoice ? "checkbox" : "radio"}
            accessibilityState={{ checked: picked || mine }}
            disabled={!choosing}
            onPress={() => toggle(option.id)}
            className={`overflow-hidden rounded-control border-2 bg-surface ${picked ? "border-brand" : "border-line"}`}
          >
            {poll.resultsVisible && (
              <View
                className="absolute inset-y-0 left-0 bg-brand/20"
                style={{ width: `${percent}%` }}
              />
            )}
            <View className="flex-row items-center justify-between gap-2 px-3 py-2.5">
              <Text className={`flex-1 text-sm text-ink ${mine ? "font-black" : "font-bold"}`}>
                {mine ? `✓ ${option.text}` : option.text}
              </Text>
              {poll.resultsVisible && (
                <Text className="text-xs font-bold text-ink-soft">
                  {t("poll.result", { percent, count: option.voteCount })}
                </Text>
              )}
            </View>
          </Pressable>
        );
      })}
      <View className="flex-row items-center justify-between gap-2">
        <Text className="text-xs text-ink-soft">
          {poll.resultsVisible
            ? t("poll.voters", { count: poll.voterCount })
            : t("poll.resultsHidden")}
        </Text>
        {choosing && (
          <Button
            testID="poll-vote"
            size="sm"
            loading={busy}
            disabled={chosen.length === 0}
            onPress={() => void vote(chosen)}
          >
            {t("poll.vote")}
          </Button>
        )}
        {open && canVote && voted && (
          <Button size="sm" variant="ghost" loading={busy} onPress={() => void vote([])}>
            {t("poll.retract")}
          </Button>
        )}
      </View>
    </View>
  );
}
