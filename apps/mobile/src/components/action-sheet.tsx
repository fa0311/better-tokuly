import type { ReactNode } from "react";
import { Modal, Pressable, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

export interface SheetAction {
  key: string;
  label: string;
  onPress(): void;
  destructive?: boolean;
}

/**
 * A list of choices rising from the bottom (Android's alerts hold three
 * buttons at most). A tap on a choice or outside closes it; `children` go
 * above the choices (a row of reactions).
 */
export function ActionSheet({
  visible,
  title,
  actions,
  onClose,
  children,
}: {
  visible: boolean;
  title?: string;
  actions: SheetAction[];
  onClose(): void;
  children?: ReactNode;
}) {
  const insets = useSafeAreaInsets();
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      <View
        testID="action-sheet"
        className="rounded-t-3xl bg-surface px-2 pt-3"
        style={{ paddingBottom: insets.bottom + 12 }}
      >
        {title && (
          <Text numberOfLines={1} className="px-4 pb-2 text-center font-black text-ink-soft">
            {title}
          </Text>
        )}
        {children}
        {actions.map((action) => (
          <Pressable
            key={action.key}
            testID={`sheet-${action.key}`}
            accessibilityRole="button"
            onPress={() => {
              onClose();
              action.onPress();
            }}
            className="rounded-2xl px-4 py-3.5 active:bg-surface-2"
          >
            <Text
              className={`text-base font-black ${action.destructive ? "text-danger" : "text-ink"}`}
            >
              {action.label}
            </Text>
          </Pressable>
        ))}
      </View>
    </Modal>
  );
}
