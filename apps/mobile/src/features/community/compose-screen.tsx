import { useQueryClient } from "@tanstack/react-query";
import { community } from "@tokuly/api";
import { communityKeys, parseTags } from "@tokuly/core/community";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Page, Spinner } from "#/components/page";
import { TextField } from "#/components/text-field";
import { errorMessage } from "#/lib/errors";
import { asAccount, boardClient, NS, useChannels, useCommunity } from "./data";

/**
 * `/g/<handle>/new`: starting a thread in one of the channels the caller
 * may post in (`?channel=` picks it). Text, tags; pictures, stamps and
 * polls are written on the web for now.
 */
export function ComposeThreadScreen({ handle, channel }: { handle: string; channel?: string }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const c = useCommunity(handle).data;
  const channels = useChannels(c?.id ?? "", Boolean(c));
  const open = (channels.data ?? []).filter((each) => each.canPost);
  const [channelId, setChannelId] = useState(channel ?? "");
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [tags, setTags] = useState("");
  const [sending, setSending] = useState(false);

  if (!c || channels.isPending) return <Spinner />;
  if (open.length === 0)
    return (
      <View className="flex-1 bg-bg">
        <EmptyState emoji="🙊" title={t("compose.noChannel")} />
      </View>
    );
  const chosen = open.find((each) => each.id === channelId) ?? open[0];

  const post = async () => {
    if (!chosen || !title.trim() || !body.trim() || sending) return;
    setSending(true);
    try {
      const answer = await asAccount(() =>
        boardClient().createThread({
          channelId: chosen.id,
          title: title.trim(),
          body: body.trim(),
          tags: parseTags(tags, 5),
        }),
      );
      if (!answer) return;
      const { thread } = answer;
      void queryClient.invalidateQueries({ queryKey: communityKeys.threadLists(c.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.channels(c.id) });
      if (thread?.state === community.PostState.HIDDEN) Alert.alert(t("compose.held"));
      router.back();
      if (thread) router.push(`/g/${c.handle}/t/${thread.shortId}`);
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: NS }));
    } finally {
      setSending(false);
    }
  };

  return (
    <Page testID="thread-compose">
      <View className="gap-2">
        <Text className="px-1 text-sm font-bold text-ink-soft">{t("compose.channel")}</Text>
        <View className="flex-row flex-wrap gap-2">
          {open.map((each) => {
            const picked = each.id === chosen?.id;
            return (
              <Pressable
                key={each.id}
                testID={`compose-channel-${each.id}`}
                accessibilityRole="radio"
                accessibilityState={{ checked: picked }}
                onPress={() => setChannelId(each.id)}
                className={`rounded-full px-3 py-1.5 ${picked ? "bg-brand" : "bg-surface"}`}
              >
                <Text className={`text-sm font-bold ${picked ? "text-on-brand" : "text-ink"}`}>
                  {`${each.emoji ? `${each.emoji} ` : ""}${each.name}`}
                </Text>
              </Pressable>
            );
          })}
        </View>
        {chosen?.anonymous && (
          <Text className="rounded-control bg-surface-2 px-3 py-2 text-xs leading-5 text-ink-soft">
            {`🎭 ${t("anonymous.body")}`}
          </Text>
        )}
      </View>
      <TextField
        testID="compose-title"
        label={t("compose.title")}
        placeholder={t("compose.titlePlaceholder")}
        maxLength={100}
        value={title}
        onChangeText={setTitle}
      />
      <TextField
        testID="compose-body"
        label={t("compose.body")}
        multiline
        maxLength={5000}
        placeholder={t("compose.bodyPlaceholder")}
        value={body}
        onChangeText={setBody}
        style={{ minHeight: 160 }}
      />
      <TextField
        testID="compose-tags"
        label={t("compose.tags")}
        hint={t("compose.tagsHint")}
        autoCapitalize="none"
        value={tags}
        onChangeText={setTags}
      />
      <Button
        testID="compose-post"
        size="lg"
        block
        loading={sending}
        // Text is what this screen has to give a thread (the server wants more than a title).
        disabled={!title.trim() || !body.trim()}
        onPress={() => void post()}
      >
        {t("compose.post")}
      </Button>
    </Page>
  );
}
