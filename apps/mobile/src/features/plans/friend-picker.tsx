import type { common } from "@tokuly/api";
import { nameOf } from "@tokuly/core/plans";
import { Check } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { KeyboardAvoidingView, Modal, Pressable, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { Spinner } from "#/components/page";
import { TextField } from "#/components/text-field";
import { useFriends } from "#/features/friends/data";
import { useTheme } from "#/lib/theme";

/**
 * Friends to invite, chosen from a list that rises from the bottom; people
 * already in the plan (`taken`) are not offered.
 */
export function FriendPicker({
  visible,
  title,
  lead,
  initial,
  taken = [],
  confirmLabel,
  busy,
  onConfirm,
  onClose,
}: {
  visible: boolean;
  title: string;
  lead?: string;
  initial: common.UserSummary[];
  taken?: string[];
  confirmLabel: string;
  busy?: boolean;
  onConfirm(chosen: common.UserSummary[]): void;
  onClose(): void;
}) {
  const { t } = useTranslation("plans");
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState<common.UserSummary[] | null>(null);
  const picked = chosen ?? initial;
  const friends = useFriends(query.trim());
  useEffect(() => {
    if (!visible) {
      setChosen(null);
      setQuery("");
    }
  }, [visible]);
  const people = (friends.data?.friends ?? [])
    .flatMap((friend) => (friend.user ? [friend.user] : []))
    .filter((user) => !taken.includes(user.id));
  const toggle = (user: common.UserSummary) =>
    setChosen(
      picked.some((other) => other.id === user.id)
        ? picked.filter((other) => other.id !== user.id)
        : [...picked, user],
    );
  const close = () => {
    setChosen(null);
    setQuery("");
    onClose();
  };

  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={close}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={close} />
      <KeyboardAvoidingView behavior="padding">
        <View
          testID="plan-friend-picker"
          className="max-h-[640px] gap-3 rounded-t-3xl bg-surface px-5 pt-5"
          style={{ paddingBottom: insets.bottom + 16 }}
        >
          <Text className="text-xl font-black text-ink">{title}</Text>
          {lead && <Text className="-mt-2 text-sm text-ink-soft">{lead}</Text>}
          <TextField
            testID="plan-friend-search"
            label={t("picker.search")}
            value={query}
            onChangeText={setQuery}
            autoCapitalize="none"
          />
          <ScrollView className="max-h-80" keyboardShouldPersistTaps="handled">
            {friends.isPending ? (
              <Spinner />
            ) : people.length === 0 ? (
              <Text className="py-6 text-center text-sm text-ink-soft">
                {query ? t("picker.noMatch") : t("picker.emptyBody")}
              </Text>
            ) : (
              people.map((user) => {
                const selected = picked.some((other) => other.id === user.id);
                return (
                  <Pressable
                    key={user.id}
                    testID={`plan-friend-${user.handle}`}
                    accessibilityRole="checkbox"
                    accessibilityState={{ checked: selected }}
                    onPress={() => toggle(user)}
                    className={`flex-row items-center gap-3 rounded-2xl px-3 py-2 ${selected ? "bg-brand-soft" : "active:bg-surface-2"}`}
                  >
                    <Avatar user={user} size="sm" />
                    <View className="flex-1">
                      <Text numberOfLines={1} className="font-black text-ink">
                        {nameOf(user)}
                      </Text>
                      <Text numberOfLines={1} className="text-xs text-ink-soft">
                        @{user.handle}
                      </Text>
                    </View>
                    {selected && <Check size={20} color={colors.brand} />}
                  </Pressable>
                );
              })
            )}
          </ScrollView>
          <Button
            testID="plan-friend-confirm"
            size="lg"
            block
            loading={busy}
            onPress={() => {
              onConfirm(picked);
            }}
          >
            {confirmLabel}
          </Button>
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}
