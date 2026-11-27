import { FlashList } from "@shopify/flash-list";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type chat, social } from "@tokuly/api";
import { chatKeys } from "@tokuly/core/chat";
import { friendQueries } from "@tokuly/core/friends";
import { router } from "expo-router";
import { Check } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, TextInput, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { conversationClient } from "./data";

const friendClient = () => api().service(social.FriendService);

/** Opens a conversation that was just made: enough to draw at once, marked old so its extras still load. */
function open(queryClient: ReturnType<typeof useQueryClient>, conversation: chat.Conversation) {
  queryClient.setQueryData(chatKeys.conversation(conversation.id), conversation, { updatedAt: 0 });
  router.replace({ pathname: "/chat/[id]", params: { id: conversation.id } });
}

/**
 * A new chat with friends: one picked opens (or starts) the direct
 * conversation with them, more make a group.
 */
export function NewChatScreen() {
  const { t } = useTranslation("chat");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<social.Friend[]>([]);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const friends = useQuery(friendQueries.list(friendClient, query.trim()));
  const pickedIds = new Set(picked.map((friend) => friend.user?.id));

  const toggle = (friend: social.Friend) =>
    setPicked((held) =>
      pickedIds.has(friend.user?.id)
        ? held.filter((other) => other.user?.id !== friend.user?.id)
        : [...held, friend],
    );

  const start = async () => {
    setBusy(true);
    try {
      if (picked.length === 1) {
        const { conversation } = await conversationClient().openDirect({
          userId: picked[0]!.user!.id,
        });
        if (conversation) open(queryClient, conversation);
      } else {
        const { conversation } = await conversationClient().createGroup({
          name: name.trim(),
          memberIds: picked.map((friend) => friend.user!.id),
        });
        if (conversation) open(queryClient, conversation);
      }
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "chat" }));
    } finally {
      setBusy(false);
    }
  };

  const list = friends.data?.friends ?? [];
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="new-chat-screen">
      <Text className="px-4 pt-3 text-sm font-bold text-ink-soft">{tm("chat.newLead")}</Text>
      <TextInput
        accessibilityLabel={t("picker.search")}
        placeholder={t("picker.search")}
        placeholderTextColor={colors["ink-faint"]}
        value={query}
        onChangeText={setQuery}
        autoCapitalize="none"
        className="mx-4 my-2 h-11 py-0 rounded-full bg-surface px-4 text-ink"
      />
      {friends.isPending ? (
        <Spinner />
      ) : (
        <FlashList
          data={list}
          keyExtractor={(friend) => friend.user?.id ?? ""}
          extraData={picked}
          renderItem={({ item }) => {
            const selected = pickedIds.has(item.user?.id);
            return (
              <Pressable
                testID={`pick-${item.user?.handle}`}
                accessibilityRole="checkbox"
                accessibilityState={{ checked: selected }}
                onPress={() => toggle(item)}
                className="flex-row items-center gap-3 border-b border-line bg-surface px-4 py-3"
              >
                <Avatar user={item.user} size="md" online={item.online} />
                <View className="min-w-0 flex-1">
                  <Text numberOfLines={1} className="font-black text-ink">
                    {item.nickname || item.user?.displayName}
                  </Text>
                  <Text numberOfLines={1} className="text-sm font-bold text-ink-soft">
                    @{item.user?.handle}
                  </Text>
                </View>
                <View
                  className={`h-7 w-7 items-center justify-center rounded-full border-2 ${
                    selected ? "border-brand bg-brand" : "border-line"
                  }`}
                >
                  {selected && <Check size={16} color={colors["on-brand"]} />}
                </View>
              </Pressable>
            );
          }}
          ListEmptyComponent={
            query ? (
              <EmptyState emoji="🔍" title={t("picker.noMatch")} />
            ) : (
              <EmptyState
                emoji="🫂"
                title={t("picker.noFriends")}
                body={t("picker.noFriendsBody")}
              />
            )
          }
        />
      )}
      {picked.length > 0 && (
        <View className="gap-2 border-t border-line bg-surface px-4 py-3">
          {picked.length > 1 && (
            <TextInput
              testID="group-name"
              accessibilityLabel={t("group.name")}
              placeholder={tm("chat.groupName")}
              placeholderTextColor={colors["ink-faint"]}
              value={name}
              onChangeText={setName}
              maxLength={50}
              className="h-11 py-0 rounded-full bg-surface-2 px-4 text-ink"
            />
          )}
          <Button
            testID="new-chat-start"
            block
            loading={busy}
            disabled={picked.length > 1 && !name.trim()}
            onPress={() => void start()}
          >
            {picked.length === 1
              ? tm("chat.startDirect", {
                  name: picked[0]!.nickname || picked[0]!.user?.displayName,
                })
              : tm("chat.startGroup", { count: picked.length + 1 })}
          </Button>
        </View>
      )}
    </SafeAreaView>
  );
}
