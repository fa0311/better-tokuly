import { Text, TextInput, type TextInputProps, View } from "react-native";
import { useTheme } from "#/lib/theme";

export interface TextFieldProps extends TextInputProps {
  label: string;
  hint?: string;
  /** Shown under the field instead of the hint, in the danger colour. */
  error?: string;
}

/** A labelled text input; the label is also what screen readers announce. */
export function TextField({ label, hint, error, ...input }: TextFieldProps) {
  const { colors } = useTheme();
  return (
    <View className="gap-1.5">
      <Text className="px-1 text-sm font-bold text-ink">{label}</Text>
      <TextInput
        accessibilityLabel={label}
        placeholderTextColor={colors["ink-faint"]}
        className="h-12 rounded-control border-2 border-line bg-surface px-4 text-base text-ink"
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
