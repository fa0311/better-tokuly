import { Check, ChevronDown } from "lucide-react-native";
import { useState } from "react";
import { Modal, Pressable, ScrollView, Switch, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useTheme } from "#/lib/theme";

/** A setting that is on or off, as a row of a `Section`: it saves the moment it moves. */
export function SwitchRow({
  label,
  hint,
  value,
  disabled,
  onChange,
  testID,
}: {
  label: string;
  hint?: string;
  value: boolean;
  disabled?: boolean;
  onChange(value: boolean): void;
  testID?: string;
}) {
  const { colors } = useTheme();
  return (
    <View
      className={`min-h-14 flex-row items-center gap-3 border-b border-line px-4 py-3 ${disabled ? "opacity-50" : ""}`}
    >
      <View className="flex-1">
        <Text className="text-base font-bold text-ink">{label}</Text>
        {hint && <Text className="text-xs text-ink-soft">{hint}</Text>}
      </View>
      <Switch
        testID={testID}
        accessibilityLabel={label}
        value={value}
        disabled={disabled}
        onValueChange={onChange}
        trackColor={{ true: colors.brand, false: colors["surface-3"] }}
        thumbColor={colors.surface}
      />
    </View>
  );
}

export interface Choice<T extends string> {
  value: T;
  label: string;
  /** What this choice means, in the sheet. */
  hint?: string;
}

/**
 * One choice among a few, as a row of a `Section`: the current one is written
 * under the label, and a tap opens the choices (with what each means) from
 * the bottom.
 */
export function ChoiceRow<T extends string>({
  label,
  hint,
  value,
  choices,
  onChange,
  testID,
}: {
  label: string;
  hint?: string;
  value: T;
  choices: readonly Choice<T>[];
  onChange(value: T): void;
  testID?: string;
}) {
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const [open, setOpen] = useState(false);
  const current = choices.find((choice) => choice.value === value);
  return (
    <>
      <Pressable
        testID={testID}
        accessibilityRole="button"
        accessibilityHint={current?.label}
        onPress={() => setOpen(true)}
        className="min-h-14 flex-row items-center gap-3 border-b border-line px-4 py-3 active:bg-surface-2"
      >
        <View className="flex-1">
          <Text className="text-base font-bold text-ink">{label}</Text>
          {hint && <Text className="text-xs text-ink-soft">{hint}</Text>}
          <Text className="pt-1 text-sm font-black text-brand">{current?.label}</Text>
        </View>
        <ChevronDown size={18} color={colors["ink-faint"]} />
      </Pressable>
      <Modal visible={open} transparent animationType="slide" onRequestClose={() => setOpen(false)}>
        <Pressable
          accessibilityRole="none"
          className="flex-1 bg-black/40"
          onPress={() => setOpen(false)}
        />
        <View
          className="max-h-[80%] rounded-t-3xl bg-surface px-2 pt-3"
          style={{ paddingBottom: insets.bottom + 12 }}
        >
          <Text className="px-4 pb-2 text-center font-black text-ink-soft">{label}</Text>
          <ScrollView>
            {choices.map((choice) => {
              const selected = choice.value === value;
              return (
                <Pressable
                  key={choice.value}
                  testID={testID && `${testID}-${choice.value}`}
                  accessibilityRole="radio"
                  accessibilityState={{ selected }}
                  onPress={() => {
                    setOpen(false);
                    if (!selected) onChange(choice.value);
                  }}
                  className={`flex-row items-center gap-3 rounded-2xl px-4 py-3 ${selected ? "bg-brand-soft" : "active:bg-surface-2"}`}
                >
                  <View className="flex-1">
                    <Text className="text-base font-black text-ink">{choice.label}</Text>
                    {choice.hint && <Text className="text-xs text-ink-soft">{choice.hint}</Text>}
                  </View>
                  {selected && <Check size={20} color={colors.brand} />}
                </Pressable>
              );
            })}
          </ScrollView>
        </View>
      </Modal>
    </>
  );
}
