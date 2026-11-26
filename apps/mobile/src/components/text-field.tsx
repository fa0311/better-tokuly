import { useState } from "react";
import { Text, TextInput, type TextInputProps, View } from "react-native";
import { useTheme } from "#/lib/theme";

export interface TextFieldProps extends TextInputProps {
  label: string;
  hint?: string;
  /** Shown under the field instead of the hint, in the danger colour. */
  error?: string;
}

/** A labelled text input; the label is also what screen readers announce. */
export function TextField({ label, hint, error, onFocus, onBlur, ...input }: TextFieldProps) {
  const { colors } = useTheme();
  const [focused, setFocused] = useState(false);
  const border = error ? "border-danger" : focused ? "border-brand" : "border-line";
  return (
    <View className="gap-1.5">
      <Text className="px-1 text-sm font-bold text-ink-soft">{label}</Text>
      <TextInput
        accessibilityLabel={label}
        placeholderTextColor={colors["ink-faint"]}
        className={`h-12 rounded-control border-2 bg-surface px-4 text-base text-ink ${border}`}
        onFocus={(event) => {
          setFocused(true);
          onFocus?.(event);
        }}
        onBlur={(event) => {
          setFocused(false);
          onBlur?.(event);
        }}
        {...input}
      />
      {(error || hint) && (
        <Text className={`px-1 text-xs font-bold ${error ? "text-danger" : "text-ink-soft"}`}>
          {error || hint}
        </Text>
      )}
    </View>
  );
}
