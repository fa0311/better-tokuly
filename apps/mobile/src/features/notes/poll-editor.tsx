import { notes } from "@tokuly/api";
import {
  POLL_CHOICE_MAX_CHARS,
  POLL_DURATIONS,
  POLL_MAX_CHOICES,
  type PollDraft,
} from "@tokuly/core/notes";
import { Plus, X } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, ScrollView, Switch, Text, TextInput, View } from "react-native";
import { useTheme } from "#/lib/theme";

const RESULTS = [
  { value: notes.PollResultsVisibility.ALWAYS, key: "resultsAlways" },
  { value: notes.PollResultsVisibility.AFTER_VOTING, key: "resultsAfterVoting" },
  { value: notes.PollResultsVisibility.AFTER_END, key: "resultsAfterEnd" },
] as const;

function Chip({ label, selected, onPress }: { label: string; selected: boolean; onPress(): void }) {
  return (
    <Pressable
      accessibilityRole="radio"
      accessibilityState={{ checked: selected }}
      onPress={onPress}
      className={`rounded-full px-3 py-1.5 ${selected ? "bg-ink" : "bg-surface-2"}`}
    >
      <Text className={`text-sm font-bold ${selected ? "text-surface" : "text-ink"}`}>{label}</Text>
    </Pressable>
  );
}

/** The poll under the text: 2–10 choices, one or several picked, how long, who sees the results when. */
export function PollEditor({
  poll,
  onChange,
  onRemove,
}: {
  poll: PollDraft;
  onChange(poll: PollDraft): void;
  onRemove(): void;
}) {
  const { t } = useTranslation("notes");
  const { colors } = useTheme();
  const setChoice = (index: number, value: string) =>
    onChange({ ...poll, choices: poll.choices.map((choice, i) => (i === index ? value : choice)) });
  return (
    <View testID="poll-editor" className="gap-3 rounded-card bg-surface p-3">
      <View className="flex-row items-center">
        <Text className="flex-1 font-black text-ink">📊 {t("compose.poll.title")}</Text>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("compose.poll.remove")}
          onPress={onRemove}
        >
          <X color={colors["ink-soft"]} size={20} />
        </Pressable>
      </View>
      {poll.choices.map((choice, index) => (
        <View key={index} className="flex-row items-center gap-2">
          <TextInput
            testID={`poll-input-${index}`}
            value={choice}
            maxLength={POLL_CHOICE_MAX_CHARS}
            onChangeText={(value) => setChoice(index, value)}
            placeholder={t("compose.poll.choice", { index: index + 1 })}
            placeholderTextColor={colors["ink-faint"]}
            className="h-10 flex-1 rounded-control bg-surface-2 px-3 text-base text-ink"
          />
          {poll.choices.length > 2 && (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t("compose.poll.removeChoice")}
              onPress={() =>
                onChange({ ...poll, choices: poll.choices.filter((_, i) => i !== index) })
              }
            >
              <X color={colors["ink-faint"]} size={18} />
            </Pressable>
          )}
        </View>
      ))}
      {poll.choices.length < POLL_MAX_CHOICES && (
        <Pressable
          testID="poll-add-choice"
          accessibilityRole="button"
          onPress={() => onChange({ ...poll, choices: [...poll.choices, ""] })}
          className="flex-row items-center gap-1 self-start"
        >
          <Plus color={colors.brand} size={16} />
          <Text className="font-bold text-brand">{t("compose.poll.addChoice")}</Text>
        </Pressable>
      )}
      <View className="flex-row items-center">
        <Text className="flex-1 font-bold text-ink">{t("compose.poll.multiple")}</Text>
        <Switch
          value={poll.multiple}
          onValueChange={(multiple) => onChange({ ...poll, multiple })}
        />
      </View>
      <Text className="text-xs font-black text-ink-soft">{t("compose.poll.duration")}</Text>
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerClassName="gap-2"
      >
        {POLL_DURATIONS.map((seconds) => (
          <Chip
            key={seconds}
            label={t(`compose.poll.durations.${seconds}`)}
            selected={poll.durationSeconds === seconds}
            onPress={() => onChange({ ...poll, durationSeconds: seconds })}
          />
        ))}
      </ScrollView>
      <Text className="text-xs font-black text-ink-soft">{t("compose.poll.results")}</Text>
      <View className="flex-row flex-wrap gap-2">
        {RESULTS.map((option) => (
          <Chip
            key={option.key}
            label={t(`compose.poll.${option.key}`)}
            selected={poll.results === option.value}
            onPress={() => onChange({ ...poll, results: option.value })}
          />
        ))}
      </View>
    </View>
  );
}
