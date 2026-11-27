import { useState } from "react";
import { Text, TextInput, type TextInputProps, View } from "react-native";
import { useTheme } from "#/lib/theme";

export interface TextFieldProps extends TextInputProps {
  label: string;
  hint?: string;
  /** Shown under the field instead of the hint, in the danger colour. */
  error?: string;
}

/**
 * A labelled text input; the label is also what screen readers announce. A
 * `multiline` one grows with its text from three lines.
 *
 * The box is never shorter than its text: on Android a field that can scroll
 * its own text keeps a drag that starts on it, so the page would not scroll
 * (`ReactEditText.onTouchEvent`). Hence the vertical padding is set (not the
 * platform's) and the height is a minimum.
 */
export function TextField({ label, hint, error, onFocus, onBlur, ...input }: TextFieldProps) {
  const { colors } = useTheme();
  const [focused, setFocused] = useState(false);
  const border = error ? "border-danger" : focused ? "border-brand" : "border-line";
  const box = input.multiline ? "min-h-28 py-3" : "min-h-12 py-0";
  return (
    <View className="gap-1.5">
      <Text className="px-1 text-sm font-bold text-ink-soft">{label}</Text>
      <TextInput
        accessibilityLabel={label}
        placeholderTextColor={colors["ink-faint"]}
        textAlignVertical={input.multiline ? "top" : "center"}
        className={`${box} rounded-control border-2 bg-surface px-4 text-base text-ink ${border}`}
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
