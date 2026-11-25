import type { common } from "@tokuly/api";
import { router } from "expo-router";
import type { ReactNode } from "react";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";

/** A person in a list: face, name and handle; a tap opens their profile. */
export function PersonRow({
  user,
  detail,
  online,
  children,
}: {
  user: common.UserSummary | undefined;
  detail?: string;
  online?: boolean;
  children?: ReactNode;
}) {
  if (!user) return null;
  return (
    <Pressable
      testID={`person-${user.handle}`}
      accessibilityRole="button"
      onPress={() => router.push({ pathname: "/u/[handle]", params: { handle: user.handle } })}
      className="flex-row items-center gap-3 border-b border-line bg-surface px-4 py-3"
    >
      <Avatar user={user} size="md" online={online} />
      <View className="min-w-0 flex-1">
        <Text numberOfLines={1} className="font-black text-ink">
          {user.displayName}
        </Text>
        <Text numberOfLines={1} className="text-sm font-bold text-ink-soft">
          @{user.handle}
          {detail ? ` · ${detail}` : ""}
        </Text>
      </View>
      {children}
    </Pressable>
  );
}
