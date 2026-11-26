import { useQuery } from "@tanstack/react-query";
import { media, notes } from "@tokuly/api";
import { durationLimit } from "@tokuly/core/errors";
import { friendQueries } from "@tokuly/core/friends";
import { clock } from "@tokuly/core/shorts";
import { Image } from "expo-image";
import { router } from "expo-router";
import { Camera, Check, Images, Trash2 } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Modal, Pressable, ScrollView, Text, TextInput, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { friendClient } from "#/features/friends/data";
import { CameraScreen } from "#/features/media/camera-screen";
import { recordingLimit, useRecordingLimits } from "#/features/media/limits";
import { pickFromLibrary, trimVideo } from "#/features/media/pick";
import { type LocalMedia, submitPost } from "#/features/media/posts";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { useTheme } from "#/lib/theme";

const MAX_ITEMS = 10;
const MAX_TITLE = 140;
const MAX_CAPTION = 200;
/** As many close friends as a story may name. */
const MAX_AUDIENCE = 150;

interface Item {
  media: LocalMedia;
  caption: string;
}

/**
 * Leaving a story (記憶): up to ten photos and videos from the library or
 * the camera, a title and a word for each, shown to every friend or only to
 * the close friends picked for it. It goes up in the background: the tray
 * shows how it is doing.
 */
export function StoryComposerScreen() {
  const { t } = useTranslation("stories");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const limit = useRecordingLimits((limits) => limits[media.AssetPurpose.STORY]);
  const [items, setItems] = useState<Item[]>([]);
  const [title, setTitle] = useState("");
  const [audience, setAudience] = useState(notes.StoryAudience.FRIENDS);
  const [picked, setPicked] = useState<string[]>([]);
  const [camera, setCamera] = useState(false);
  const [busy, setBusy] = useState(false);
  const friends = useQuery({
    ...friendQueries.list(friendClient, ""),
    enabled: audience === notes.StoryAudience.CLOSE_FRIENDS,
  });

  /** Videos longer than the server takes are cut down first (or left out). */
  const fit = async (added: LocalMedia[]): Promise<LocalMedia[]> => {
    const fitted: LocalMedia[] = [];
    for (const item of added) {
      if (
        item.kind !== "video" ||
        !item.duration ||
        item.duration <= recordingLimit(media.AssetPurpose.STORY)
      ) {
        fitted.push(item);
        continue;
      }
      const trimmed = await trimVideo(item, recordingLimit(media.AssetPurpose.STORY));
      if (trimmed) fitted.push(trimmed);
    }
    return fitted;
  };
  const add = async (added: LocalMedia[]) => {
    const fitted = await fit(added);
    setItems((held) =>
      [...held, ...fitted.map((media) => ({ media, caption: "" }))].slice(0, MAX_ITEMS),
    );
  };
  const pick = async () => {
    const result = await pickFromLibrary({
      kinds: ["images", "videos"],
      limit: MAX_ITEMS - items.length,
    });
    if (!result) return;
    if ("denied" in result) return Alert.alert(tm("chat.photoDenied"));
    await add(result.media);
  };

  const post = async () => {
    setBusy(true);
    try {
      await submitPost(
        {
          kind: "story",
          story: {
            title: title.trim(),
            captions: items.map((item) => item.caption.trim()),
            audience,
            audienceUserIds: picked,
          },
        },
        items.map((item) => item.media),
      );
      router.back();
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "stories" }));
    } finally {
      setBusy(false);
    }
  };

  const closeOnly = audience === notes.StoryAudience.CLOSE_FRIENDS;
  const canPost = items.length > 0 && (!closeOnly || picked.length > 0) && !busy;
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="story-composer">
      <ScrollView
        contentContainerClassName="gap-4 px-4 pt-4 pb-12"
        keyboardShouldPersistTaps="handled"
      >
        <Text className="text-sm text-ink-soft">{t("create.lead")}</Text>
        <View className="flex-row gap-3">
          <Button
            testID="story-pick"
            variant="secondary"
            icon={<Images color={colors.ink} size={18} />}
            disabled={items.length >= MAX_ITEMS}
            onPress={() => void pick()}
          >
            {items.length
              ? t("create.addMore", { count: MAX_ITEMS - items.length })
              : t("create.pick")}
          </Button>
          <Button
            testID="story-camera"
            variant="secondary"
            icon={<Camera color={colors.ink} size={18} />}
            disabled={items.length >= MAX_ITEMS}
            onPress={() => setCamera(true)}
          >
            {tm("chat.camera")}
          </Button>
        </View>
        <Text className="text-xs text-ink-faint">
          {tm("stories.limits", { count: MAX_ITEMS, limit: durationLimit(i18n, limit) })}
        </Text>
        {items.map((item, index) => (
          <View
            key={`${item.media.uri}-${index}`}
            className="flex-row items-center gap-3 rounded-card bg-surface p-2"
          >
            {item.media.kind === "image" ? (
              <Image
                source={item.media.uri}
                style={{ width: 64, height: 96, borderRadius: 12 }}
                contentFit="cover"
              />
            ) : (
              <View
                className="items-center justify-center rounded-xl bg-surface-3"
                style={{ width: 64, height: 96 }}
              >
                <Text className="text-2xl">🎬</Text>
                {item.media.duration ? (
                  <Text className="text-xs font-bold text-ink-soft">
                    {clock(item.media.duration)}
                  </Text>
                ) : null}
              </View>
            )}
            <TextInput
              testID={`story-caption-${index}`}
              value={item.caption}
              maxLength={MAX_CAPTION}
              onChangeText={(caption) =>
                setItems((held) =>
                  held.map((other, at) => (at === index ? { ...other, caption } : other)),
                )
              }
              placeholder={t("create.caption")}
              placeholderTextColor={colors["ink-faint"]}
              className="flex-1 text-base text-ink"
              multiline
            />
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={tm("stories.remove")}
              onPress={() => setItems((held) => held.filter((_, at) => at !== index))}
              className="p-2"
            >
              <Trash2 color={colors["ink-soft"]} size={20} />
            </Pressable>
          </View>
        ))}
        <View className="gap-1">
          <Text className="px-1 text-xs font-black text-ink-soft">{t("create.titleLabel")}</Text>
          <TextInput
            testID="story-title"
            value={title}
            maxLength={MAX_TITLE}
            onChangeText={setTitle}
            placeholder={t("create.titlePlaceholder")}
            placeholderTextColor={colors["ink-faint"]}
            className="rounded-control bg-surface px-4 py-3 text-base text-ink"
          />
        </View>
        <View className="gap-2">
          <Text className="px-1 text-xs font-black text-ink-soft">{t("create.audience")}</Text>
          <View className="flex-row gap-2">
            {[
              {
                value: notes.StoryAudience.FRIENDS,
                label: t("create.everyFriend"),
                id: "story-audience-friends",
              },
              {
                value: notes.StoryAudience.CLOSE_FRIENDS,
                label: picked.length
                  ? t("create.closeCount", { count: picked.length })
                  : t("create.close"),
                id: "story-audience-close",
              },
            ].map((option) => (
              <Pressable
                key={option.id}
                testID={option.id}
                accessibilityRole="radio"
                accessibilityState={{ checked: audience === option.value }}
                onPress={() => setAudience(option.value)}
                className={`rounded-full px-4 py-2 ${audience === option.value ? "bg-ink" : "bg-surface"}`}
              >
                <Text
                  className={`font-black ${audience === option.value ? "text-surface" : "text-ink"}`}
                >
                  {option.label}
                </Text>
              </Pressable>
            ))}
          </View>
          {closeOnly && (
            <View className="overflow-hidden rounded-card bg-surface">
              {(friends.data?.friends ?? []).map((friend) => {
                const id = friend.user?.id ?? "";
                const on = picked.includes(id);
                return (
                  <Pressable
                    key={id}
                    accessibilityRole="checkbox"
                    accessibilityState={{ checked: on }}
                    onPress={() =>
                      setPicked((held) =>
                        on
                          ? held.filter((other) => other !== id)
                          : held.length < MAX_AUDIENCE
                            ? [...held, id]
                            : held,
                      )
                    }
                    className="flex-row items-center gap-3 px-4 py-2.5"
                  >
                    <Avatar user={friend.user} size="sm" />
                    <Text className="flex-1 font-bold text-ink">{friend.user?.displayName}</Text>
                    {on && <Check color={colors.brand} size={20} />}
                  </Pressable>
                );
              })}
              {friends.data?.friends.length === 0 && (
                <Text className="px-4 py-3 text-sm text-ink-soft">{t("create.noFriends")}</Text>
              )}
            </View>
          )}
        </View>
        <Button
          testID="story-post"
          block
          size="lg"
          loading={busy}
          disabled={!canPost}
          onPress={() => void post()}
        >
          {t("create.post")}
        </Button>
      </ScrollView>
      <Modal visible={camera} animationType="slide" onRequestClose={() => setCamera(false)}>
        <CameraScreen
          limitSeconds={limit}
          photos
          onClose={() => setCamera(false)}
          onCapture={(captured) => {
            setCamera(false);
            void add([captured]);
          }}
        />
      </Modal>
    </SafeAreaView>
  );
}
