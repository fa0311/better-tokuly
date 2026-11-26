import { common } from "@tokuly/api";
import {
  AUDIENCE_OPTIONS,
  type Audience,
  type AudienceOption,
  audienceOption,
} from "@tokuly/core/notes";
import { Check } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Modal, Pressable, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { useTheme } from "#/lib/theme";
import { useCustomCircles, useDirectCandidates } from "./data";

interface Choice {
  id: string;
  option: AudienceOption;
  label: string;
  hint: string;
  audience: Audience;
}

/**
 * Who a note is for, as a list of choices each saying who that is: the
 * fixed audiences, each of the viewer's custom circles, named friends
 * (picked below it) and only me. A reply cannot be wider than the note it
 * answers: wider choices are off (`minRank`).
 */
export function AudienceSheet({
  value,
  minRank = 0,
  onChange,
  onClose,
}: {
  value: Audience | undefined;
  minRank?: number;
  onChange(audience: Audience): void;
  onClose(): void;
}) {
  const { t } = useTranslation("notes");
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const circles = useCustomCircles(true);
  const current = value ? audienceOption(value) : undefined;
  const direct = current?.key === "direct";
  const friends = useDirectCandidates(direct);
  const people = new Set(value?.directUserIds ?? []);

  const choices: Choice[] = AUDIENCE_OPTIONS.flatMap((option): Choice[] => {
    if (option.key === "circle")
      return (circles.data ?? []).map((circle) => ({
        id: `circle:${circle.id}`,
        option,
        label: circle.emoji ? `${circle.emoji} ${circle.name}` : circle.name,
        hint: t("visibility.circle.hint"),
        audience: { visibility: common.Visibility.CIRCLE, circleId: circle.id },
      }));
    return [
      {
        id: option.key,
        option,
        label: t(`visibility.${option.key}.label`),
        hint: t(`visibility.${option.key}.hint`),
        audience:
          option.key === "direct"
            ? {
                visibility: common.Visibility.PRIVATE,
                directUserIds: value?.directUserIds ?? [],
                direct: true,
              }
            : { visibility: option.value },
      },
    ];
  });
  const selectedId = current?.key === "circle" ? `circle:${value?.circleId}` : current?.key;

  return (
    <Modal visible transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      <View
        testID="audience-sheet"
        className="max-h-[80%] gap-3 rounded-t-3xl bg-surface px-4 pt-4"
        style={{ paddingBottom: insets.bottom + 12 }}
      >
        <Text className="text-center text-base font-black text-ink">{t("visibility.title")}</Text>
        {minRank > 0 && (
          <Text className="text-center text-xs text-ink-soft">{t("visibility.replyHint")}</Text>
        )}
        <ScrollView contentContainerClassName="gap-1.5" accessibilityRole="radiogroup">
          {choices.map((choice) => {
            const selected = choice.id === selectedId;
            const disabled = choice.option.rank < minRank;
            return (
              <View key={choice.id} className="gap-1.5">
                <Pressable
                  testID={`audience-${choice.id}`}
                  accessibilityRole="radio"
                  accessibilityState={{ checked: selected, disabled }}
                  disabled={disabled}
                  onPress={() => onChange(choice.audience)}
                  className={`flex-row items-center gap-3 rounded-2xl border-2 px-3 py-2.5 ${selected ? "border-ink bg-brand-soft" : "border-line"}`}
                  style={{ opacity: disabled ? 0.4 : 1 }}
                >
                  <View className="flex-1">
                    <Text className="font-black text-ink">{choice.label}</Text>
                    <Text className="text-xs text-ink-soft">{choice.hint}</Text>
                  </View>
                  {selected && <Check color={colors.brand} size={20} strokeWidth={3} />}
                </Pressable>
                {selected && choice.id === "direct" && (
                  <View className="ml-4 gap-1" accessibilityLabel={t("visibility.direct.pick")}>
                    {(friends.data ?? []).length === 0 && (
                      <Text className="text-sm text-ink-soft">
                        {t("visibility.direct.noFriends")}
                      </Text>
                    )}
                    {(friends.data ?? []).map((friend) => {
                      const on = people.has(friend.id);
                      return (
                        <Pressable
                          key={friend.id}
                          testID={`audience-person-${friend.handle}`}
                          accessibilityRole="checkbox"
                          accessibilityState={{ checked: on }}
                          onPress={() => {
                            const next = new Set(people);
                            if (on) next.delete(friend.id);
                            else next.add(friend.id);
                            onChange({
                              visibility: common.Visibility.PRIVATE,
                              directUserIds: [...next],
                              direct: true,
                            });
                          }}
                          className="flex-row items-center gap-2 rounded-xl px-2 py-2"
                        >
                          <View
                            className={`size-5 items-center justify-center rounded border-2 border-ink ${on ? "bg-lime" : ""}`}
                          >
                            {on && <Check color="#17122b" size={12} strokeWidth={4} />}
                          </View>
                          <Text className="font-bold text-ink">
                            {friend.displayName || friend.handle}
                          </Text>
                          <Text className="text-xs text-ink-faint">@{friend.handle}</Text>
                        </Pressable>
                      );
                    })}
                  </View>
                )}
              </View>
            );
          })}
        </ScrollView>
        <Button testID="audience-done" block onPress={onClose}>
          {t("visibility.done")}
        </Button>
      </View>
    </Modal>
  );
}
