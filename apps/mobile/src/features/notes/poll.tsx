import { type notes, toMillis } from "@tokuly/api";
import { canVote, pollRemaining, pollShares } from "@tokuly/core/notes";
import { Check } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { useTheme } from "#/lib/theme";
import { useVote } from "./actions";

/**
 * A note's poll: the choices to vote for, or the results as bars when the
 * reader may see them (the server says when). Votes are final and private.
 */
export function NotePoll({ note, poll }: { note: notes.Note; poll: notes.Poll }) {
  const { t } = useTranslation("notes");
  const { colors } = useTheme();
  const vote = useVote();
  const [picked, setPicked] = useState<number[]>([]);
  const [sending, setSending] = useState(false);
  const open = canVote(poll, true);
  const shares = pollShares(poll);

  const send = async (choices: number[]) => {
    setSending(true);
    await vote(note, choices);
    setSending(false);
  };

  const [remainingKey, remainingCount] = pollRemaining(toMillis(poll.endsAt), Date.now());
  return (
    <View testID="note-poll" className="gap-1.5">
      {poll.choices.map((choice, index) => {
        if (open) {
          const selected = picked.includes(index);
          return (
            <Pressable
              key={index}
              testID={`poll-choice-${index}`}
              accessibilityRole={poll.multiple ? "checkbox" : "button"}
              accessibilityState={{ checked: poll.multiple ? selected : undefined }}
              disabled={sending}
              onPress={() => {
                if (!poll.multiple) void send([index]);
                else setPicked(selected ? picked.filter((i) => i !== index) : [...picked, index]);
              }}
              className={`flex-row items-center gap-2 rounded-xl border-2 px-3 py-2.5 ${selected ? "border-ink bg-brand-soft" : "border-line"}`}
            >
              {poll.multiple && (
                <View
                  className={`size-4 items-center justify-center rounded border-2 border-ink ${selected ? "bg-lime" : ""}`}
                >
                  {selected && <Check color="#17122b" size={10} strokeWidth={4} />}
                </View>
              )}
              <Text className="flex-1 font-bold text-ink">{choice.text}</Text>
            </Pressable>
          );
        }
        const mine = poll.myChoices.includes(index);
        return (
          <View key={index} className="overflow-hidden rounded-xl border-2 border-line px-3 py-2.5">
            {poll.resultsVisible && (
              <View
                className={`absolute inset-y-0 left-0 ${mine ? "bg-lime/60" : "bg-surface-3"}`}
                style={{ width: `${shares[index] ?? 0}%` }}
              />
            )}
            <View className="flex-row items-center gap-2">
              {mine && (
                <Check
                  color={colors.ink}
                  size={16}
                  strokeWidth={3}
                  accessibilityLabel={t("poll.mine")}
                />
              )}
              <Text className="flex-1 font-bold text-ink">{choice.text}</Text>
              {poll.resultsVisible && (
                <Text className="font-black text-ink">{shares[index] ?? 0}%</Text>
              )}
            </View>
          </View>
        );
      })}
      {open && poll.multiple && (
        <Button
          testID="poll-vote"
          size="sm"
          disabled={picked.length === 0}
          loading={sending}
          onPress={() => void send([...picked].sort((a, b) => a - b))}
        >
          {t("poll.vote")}
        </Button>
      )}
      <Text className="text-xs text-ink-faint">
        {poll.resultsVisible
          ? t("poll.voters", { count: Number(poll.voterCount) })
          : t("poll.hidden")}
        {" · "}
        {t(remainingKey, { count: remainingCount })}
      </Text>
    </View>
  );
}
