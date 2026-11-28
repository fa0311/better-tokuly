import { useQuery } from "@tanstack/react-query";
import { common, media, social } from "@tokuly/api";
import { durationLimit } from "@tokuly/core/errors";
import { circleKeys, isCloseFriends } from "@tokuly/core/friends";
import { clock, completeToken, MAX_CAPTION, tokenAtCaret } from "@tokuly/core/shorts";
import { router } from "expo-router";
import { Camera, Film, Lock } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Modal, Pressable, ScrollView, Switch, Text, TextInput, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { useAccountLocked } from "#/features/friends/data";
import { CameraScreen } from "#/features/media/camera-screen";
import { useRecordingLimits } from "#/features/media/limits";
import { pickFromLibrary } from "#/features/media/pick";
import { type LocalMedia, submitPost } from "#/features/media/posts";
import { useTrimmer } from "#/features/media/trim-screen";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { useTheme } from "#/lib/theme";
import { useHashtagSuggestions } from "./data";

const { Visibility } = common;

/** Who a short can be shown to (`#96`/`#99`: the audience API), in the order they are offered. */
const AUDIENCES = [
  { value: Visibility.PUBLIC, key: "public" },
  { value: Visibility.FOLLOWERS, key: "followers" },
  { value: Visibility.FRIENDS, key: "friends" },
  { value: Visibility.CLOSE_FRIENDS, key: "closeFriends" },
  { value: Visibility.CIRCLE, key: "circle" },
  { value: Visibility.PRIVATE, key: "private" },
] as const;

const circleClient = () => api().service(social.CircleService);

/**
 * Posting a short: a vertical video recorded here or picked from the
 * library, cut down to the server's limit when it is longer, a caption with
 * hashtags (suggested while typing), who sees it, and whether it takes
 * comments. It goes up in the background; "my shorts" shows how it is doing.
 */
export function CreateShortScreen() {
  const { t } = useTranslation("shorts");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const locked = useAccountLocked();
  const limit = useRecordingLimits((limits) => limits[media.AssetPurpose.SHORT]);
  const [video, setVideo] = useState<LocalMedia | null>(null);
  const [caption, setCaption] = useState("");
  const [caret, setCaret] = useState(0);
  const [visibility, setVisibility] = useState<common.Visibility>(Visibility.FRIENDS);
  const [circleId, setCircleId] = useState("");
  const [comments, setComments] = useState(true);
  const [camera, setCamera] = useState(false);
  const [busy, setBusy] = useState(false);
  const trimmer = useTrimmer();
  const token = tokenAtCaret(caption, caret);
  const suggestions = useHashtagSuggestions(token?.kind === "hashtag" ? token.query : "");
  const circles = useQuery({
    queryKey: circleKeys.list,
    queryFn: async () => (await circleClient().listCircles({})).circles,
    enabled: visibility === Visibility.CIRCLE,
  });

  /** A video longer than the server takes is cut down first (or not taken). */
  const take = async (picked: LocalMedia) => {
    if (picked.duration && picked.duration > limit) {
      const trimmed = await trimmer.trim(picked, limit);
      if (!trimmed) return;
      setVideo(trimmed);
    } else setVideo(picked);
  };
  const pick = async () => {
    try {
      const result = await pickFromLibrary({ kinds: ["videos"], limit: 1 });
      if (!result) return;
      if ("denied" in result) return Alert.alert(tm("chat.photoDenied"));
      if (result.media[0]) await take(result.media[0]);
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "shorts" }));
    }
  };

  const post = async () => {
    if (!video) return;
    setBusy(true);
    try {
      await submitPost(
        {
          kind: "short",
          short: { caption: caption.trim(), visibility, circleId, commentsEnabled: comments },
        },
        [video],
      );
      router.replace("/shorts/mine");
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "shorts" }));
    } finally {
      setBusy(false);
    }
  };

  const canPost = Boolean(video) && (visibility !== Visibility.CIRCLE || circleId) && !busy;
  const customCircles = (circles.data ?? []).filter((circle) => !isCloseFriends(circle));
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="short-composer">
      <ScrollView
        contentContainerClassName="gap-4 px-4 pt-4 pb-12"
        keyboardShouldPersistTaps="handled"
      >
        {video ? (
          <View className="flex-row items-center gap-3 rounded-card bg-surface p-3">
            <Text className="text-3xl">🎬</Text>
            <Text testID="short-video" className="flex-1 font-bold text-ink" numberOfLines={1}>
              {video.duration ? clock(video.duration) : video.name}
            </Text>
            <Button size="sm" variant="ghost" onPress={() => setVideo(null)}>
              {t("create.remove")}
            </Button>
          </View>
        ) : (
          <View className="gap-3">
            <Text className="text-sm text-ink-soft">
              {tm("shorts.pickHint", { limit: durationLimit(i18n, limit) })}
            </Text>
            <View className="flex-row gap-3">
              <Button
                testID="short-pick"
                variant="secondary"
                icon={<Film color={colors.ink} size={18} />}
                onPress={() => void pick()}
              >
                {t("create.choose")}
              </Button>
              <Button
                testID="short-record"
                variant="secondary"
                icon={<Camera color={colors.ink} size={18} />}
                onPress={() => setCamera(true)}
              >
                {tm("shorts.record")}
              </Button>
            </View>
          </View>
        )}
        <View className="gap-1">
          <Text className="px-1 text-xs font-black text-ink-soft">{t("form.caption")}</Text>
          <TextInput
            testID="short-caption-input"
            value={caption}
            maxLength={MAX_CAPTION}
            multiline
            onChangeText={setCaption}
            onSelectionChange={(event) => setCaret(event.nativeEvent.selection.end)}
            placeholder={t("form.captionPlaceholder")}
            placeholderTextColor={colors["ink-faint"]}
            className="min-h-[96px] rounded-control bg-surface px-4 py-3 text-base text-ink"
          />
          <Text className="px-1 text-right text-xs text-ink-faint">
            {t("form.captionHint", { count: caption.length, max: MAX_CAPTION })}
          </Text>
          {token?.kind === "hashtag" && (suggestions.data?.length ?? 0) > 0 && (
            <View className="flex-row flex-wrap gap-2" accessibilityLabel={t("form.suggestions")}>
              {suggestions.data!.map((hashtag) => (
                <Pressable
                  key={hashtag.tag}
                  accessibilityRole="button"
                  onPress={() => {
                    const done = completeToken(caption, token.start, caret, `#${hashtag.tag}`);
                    setCaption(done.text);
                    setCaret(done.caret);
                  }}
                  className="rounded-full bg-brand-soft px-3 py-1.5"
                >
                  <Text className="font-bold text-brand-strong">#{hashtag.tag}</Text>
                </Pressable>
              ))}
            </View>
          )}
        </View>
        <View className="gap-2">
          <Text className="px-1 text-xs font-black text-ink-soft">{t("form.visibility")}</Text>
          <View className="overflow-hidden rounded-card bg-surface">
            {AUDIENCES.map((option) => (
              <Pressable
                key={option.key}
                testID={`short-visibility-${option.key}`}
                accessibilityRole="radio"
                accessibilityState={{ checked: visibility === option.value }}
                onPress={() => setVisibility(option.value)}
                className={`gap-0.5 px-4 py-3 ${visibility === option.value ? "bg-brand-soft" : ""}`}
              >
                <Text className="font-black text-ink">{t(`visibility.${option.key}.label`)}</Text>
                <Text className="text-xs text-ink-soft">
                  {/* Public, from a locked account, is not public: it never says "everyone, search too". */}
                  {locked && option.value === Visibility.PUBLIC
                    ? t("form.lockedNote")
                    : t(`visibility.${option.key}.hint`)}
                </Text>
              </Pressable>
            ))}
          </View>
          {locked && (
            <View testID="locked-note" className="flex-row items-start gap-2 px-1">
              <Lock color={colors["ink-soft"]} size={16} />
              <Text className="flex-1 text-sm text-ink-soft">{t("form.lockedNote")}</Text>
            </View>
          )}
          {visibility === Visibility.CIRCLE && (
            <View className="gap-2">
              <Text className="px-1 text-xs font-black text-ink-soft">{t("form.circle")}</Text>
              {customCircles.length === 0 && !circles.isPending ? (
                <Text className="text-sm text-ink-soft">{t("form.circleNone")}</Text>
              ) : (
                <View className="flex-row flex-wrap gap-2">
                  {customCircles.map((circle) => (
                    <Pressable
                      key={circle.id}
                      accessibilityRole="radio"
                      accessibilityState={{ checked: circleId === circle.id }}
                      onPress={() => setCircleId(circle.id)}
                      className={`rounded-full px-3 py-2 ${circleId === circle.id ? "bg-ink" : "bg-surface"}`}
                    >
                      <Text
                        className={`font-bold ${circleId === circle.id ? "text-surface" : "text-ink"}`}
                      >
                        {circle.emoji} {circle.name}
                      </Text>
                    </Pressable>
                  ))}
                </View>
              )}
            </View>
          )}
        </View>
        <View className="flex-row items-center gap-3 rounded-card bg-surface px-4 py-3">
          <View className="flex-1">
            <Text className="font-black text-ink">{t("form.comments")}</Text>
            <Text className="text-xs text-ink-soft">{t("form.commentsHint")}</Text>
          </View>
          <Switch value={comments} onValueChange={setComments} />
        </View>
        <Button
          testID="short-post"
          block
          size="lg"
          loading={busy}
          disabled={!canPost}
          onPress={() => void post()}
        >
          {t("create.post")}
        </Button>
      </ScrollView>
      {trimmer.element}
      <Modal visible={camera} animationType="slide" onRequestClose={() => setCamera(false)}>
        <CameraScreen
          limitSeconds={limit}
          photos={false}
          onClose={() => setCamera(false)}
          onCapture={(captured) => {
            setCamera(false);
            void take(captured);
          }}
        />
      </Modal>
    </SafeAreaView>
  );
}
