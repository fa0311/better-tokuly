import { useQueryClient } from "@tanstack/react-query";
import { type common, media, moments } from "@tokuly/api";
import {
  momentKeys,
  REALMOJI_FACES,
  reactionBlocker,
  realmojiGlyph,
  setMyReaction,
} from "@tokuly/core/moments";
import { randomUUID } from "expo-crypto";
import { Image } from "expo-image";
import * as ImagePicker from "expo-image-picker";
import { Sticker } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ActivityIndicator, Alert, Modal, Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { CameraScreen } from "#/features/media/camera-screen";
import type { LocalMedia } from "#/features/media/posts";
import { type UploadState, uploadFile } from "#/features/media/upload";
import {
  type Account,
  AccountChanged,
  currentAccount,
  ensureActive,
} from "#/features/session/account";
import { StampPicker } from "#/features/stamps/picker";
import { StampImage } from "#/features/stamps/stamp-image";
import { mediaUrl } from "#/lib/config";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { forgetFiles, keepFile } from "#/platform/transfer";
import { useMomentRealmojis } from "./data";

class PhotoPreparationError extends Error {}

const FACE_KEYS: Record<moments.RealmojiEmoji, string> = {
  [moments.RealmojiEmoji.UNSPECIFIED]: "",
  [moments.RealmojiEmoji.THUMBS_UP]: "thumbsUp",
  [moments.RealmojiEmoji.SMILE]: "smile",
  [moments.RealmojiEmoji.SURPRISED]: "surprised",
  [moments.RealmojiEmoji.HEART_EYES]: "heartEyes",
  [moments.RealmojiEmoji.LAUGHING]: "laughing",
  [moments.RealmojiEmoji.INSTANT]: "instant",
};

const CHOICES = [...REALMOJI_FACES, moments.RealmojiEmoji.INSTANT];

/** The newest RealMoji faces and the total reaction count for a Moment. */
export function MomentReactionStrip({
  moment,
  onPress,
}: {
  moment: moments.Moment;
  onPress?: () => void;
}) {
  const { t } = useTranslation("moments");
  if (moment.reactionCount === 0) return null;
  const label = t("card.reactions", { count: moment.reactionCount });
  const contents = (
    <>
      <View className="flex-row">
        {moment.reactions.slice(0, 5).map((reaction, index) => (
          <MomentReactionFace key={reaction.id} reaction={reaction} index={index} />
        ))}
      </View>
      <Text className="text-xs font-bold text-ink-soft">{label}</Text>
      {onPress ? <Text className="text-xs font-bold text-brand">{t("card.seeAll")}</Text> : null}
    </>
  );

  if (onPress) {
    return (
      <Pressable
        testID="moment-reactions-open"
        accessibilityRole="button"
        accessibilityLabel={t("reactions.open", { count: moment.reactionCount })}
        onPress={onPress}
        className="flex-row items-center gap-2"
      >
        {contents}
      </Pressable>
    );
  }

  return (
    <View accessibilityLabel={label} className="flex-row items-center gap-2">
      {contents}
    </View>
  );
}

export function MomentReactionFace({
  reaction,
  index,
  size = 36,
}: {
  reaction: moments.MomentReaction;
  index?: number;
  size?: number;
}) {
  const value = reaction.reaction;
  const imageUrl = value.case === "realmoji" ? mediaUrl(value.value.image?.url) : undefined;
  return (
    <View
      style={{ marginLeft: index === undefined || index === 0 ? 0 : -6, width: size, height: size }}
      className="relative items-center justify-center overflow-hidden rounded-full border-2 border-surface bg-surface-2"
    >
      {value.case === "stamp" ? (
        <StampImage stamp={value.value} size={size - 6} />
      ) : imageUrl ? (
        <Image
          source={{ uri: imageUrl }}
          contentFit="cover"
          style={{ width: size, height: size }}
        />
      ) : (
        <Avatar user={reaction.user} size={size > 36 ? "md" : "sm"} />
      )}
      {value.case === "realmoji" && (
        <Text className="absolute -right-0.5 -bottom-1 text-xs">
          {realmojiGlyph(value.value.emoji)}
        </Text>
      )}
    </View>
  );
}

/** Reacts to an unlocked friend's Moment with a saved face or a selfie taken now. */
export function MomentReactionPicker({ moment }: { moment: moments.Moment }) {
  const { t } = useTranslation("moments");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const library = useMomentRealmojis();
  const [taking, setTaking] = useState<moments.RealmojiEmoji | null>(null);
  const [stampPickerVisible, setStampPickerVisible] = useState(false);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const controller = useRef<AbortController | null>(null);
  const draftId = useRef(randomUUID()).current;
  const accountAtCapture = currentAccount();
  const mine = moment.myReaction?.reaction;
  const pictures = new Map(library.data?.map((entry) => [entry.emoji, entry.image]));
  const blocker = reactionBlocker(moment);

  useEffect(
    () => () => {
      controller.current?.abort();
      forgetFiles(draftId);
    },
    [draftId],
  );

  const runForAccount = async (
    account: Account | null,
    work: (account: Account, signal: AbortSignal) => Promise<void>,
  ) => {
    if (!account || busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    const abort = new AbortController();
    controller.current = abort;
    try {
      ensureActive(account);
      await work(account, abort.signal);
    } catch (error) {
      if (!(error instanceof AccountChanged) && !abort.signal.aborted) {
        Alert.alert(
          t("react.failed"),
          error instanceof PhotoPreparationError
            ? t("react.photoPreparationFailed")
            : errorMessage(error),
        );
      }
    } finally {
      controller.current = null;
      busyRef.current = false;
      setBusy(false);
    }
  };

  const sendReaction = async (
    account: Account,
    signal: AbortSignal,
    reaction: moments.ReactRequest["reaction"],
    assetId?: string,
  ) => {
    const response = await account.api.service(moments.MomentReactionService).react(
      {
        momentId: moment.id,
        reaction,
        assetId,
      },
      { signal },
    );
    ensureActive(account);
    if (response.reaction) setMyReaction(queryClient, moment.id, response.reaction);
  };

  const removeReaction = () =>
    void runForAccount(currentAccount(), async (account, signal) => {
      await account.api
        .service(moments.MomentReactionService)
        .removeReaction({ momentId: moment.id }, { signal });
      ensureActive(account);
      setMyReaction(queryClient, moment.id, undefined);
    });

  const uploadAndReact = async (
    account: Account,
    signal: AbortSignal,
    emoji: moments.RealmojiEmoji,
    file: LocalMedia,
  ) => {
    try {
      const filename = file.name.split(/[\\/]/).pop() || "photo.jpg";
      const name = `realmoji-${emoji}-${filename}`;
      let kept: Awaited<ReturnType<typeof keepFile>>;
      try {
        kept = await keepFile(draftId, file.uri, name);
      } catch {
        throw new PhotoPreparationError();
      }
      ensureActive(account);
      const start: UploadState = {
        uri: kept.uri,
        name,
        contentType: file.contentType,
        size: kept.size,
      };
      const asset = await uploadFile(account, media.AssetPurpose.REALMOJI, start, {
        until: "ready",
        signal,
        save: () => {},
        onProgress: () => {},
      });
      ensureActive(account);
      if (emoji !== moments.RealmojiEmoji.INSTANT) {
        await account.api
          .service(moments.MomentReactionService)
          .setRealmoji({ emoji, assetId: asset.id }, { signal });
        ensureActive(account);
        void queryClient.invalidateQueries({ queryKey: momentKeys.realmojis });
      }
      await sendReaction(
        account,
        signal,
        { case: "realmoji", value: emoji },
        emoji === moments.RealmojiEmoji.INSTANT ? asset.id : undefined,
      );
    } finally {
      forgetFiles(draftId);
    }
  };

  const chooseFace = (emoji: moments.RealmojiEmoji) => {
    if (busy || library.isPending) return;
    if (mine?.case === "realmoji" && mine.value.emoji === emoji) return removeReaction();
    if (emoji === moments.RealmojiEmoji.INSTANT || !pictures.has(emoji)) {
      setTaking(emoji);
      return;
    }
    void runForAccount(currentAccount(), (account, signal) =>
      sendReaction(account, signal, { case: "realmoji", value: emoji }),
    );
  };

  const chooseStamp = (stamp: common.StampRef) => {
    if (busy) return;
    if (mine?.case === "stamp" && mine.value.id === stamp.id) return removeReaction();
    void runForAccount(currentAccount(), (account, signal) =>
      sendReaction(account, signal, { case: "stampId", value: stamp.id }),
    );
  };

  const pickSelfie = async (emoji: moments.RealmojiEmoji) => {
    const account = currentAccount();
    if (!account) return;
    try {
      const permission = await ImagePicker.requestMediaLibraryPermissionsAsync();
      ensureActive(account);
      if (!permission.granted) {
        Alert.alert(t("composer.photoPermission"));
        return;
      }
      const result = await ImagePicker.launchImageLibraryAsync({
        mediaTypes: ["images"],
        quality: 0.9,
        exif: false,
        preferredAssetRepresentationMode:
          ImagePicker.UIImagePickerPreferredAssetRepresentationMode.Compatible,
      });
      ensureActive(account);
      if (result.canceled || !result.assets?.length) return;
      const selected = result.assets[0];
      if (!selected) return;
      const contentType = selected.mimeType ?? "image/jpeg";
      const extension = contentType === "image/png" ? "png" : "jpg";
      const file: LocalMedia = {
        uri: selected.uri,
        kind: "image",
        name: selected.fileName ?? `realmoji-${extension}`,
        contentType,
      };
      void runForAccount(account, (active, signal) => uploadAndReact(active, signal, emoji, file));
    } catch (error) {
      if (!(error instanceof AccountChanged)) Alert.alert(t("composer.captureFailed"));
    }
  };

  if (blocker) return null;

  return (
    <View className="gap-2">
      <View className="flex-row flex-wrap items-center gap-2" accessibilityLabel={t("react.label")}>
        {CHOICES.map((emoji) => {
          const image = pictures.get(emoji);
          const imageSource = image?.url ? mediaUrl(image.url) : undefined;
          const chosen = mine?.case === "realmoji" && mine.value.emoji === emoji;
          const label = `${realmojiGlyph(emoji)} ${t(`realmoji.${FACE_KEYS[emoji]}`)}`;
          return (
            <Pressable
              key={emoji}
              testID={`moment-react-${emoji}`}
              accessibilityRole="button"
              accessibilityLabel={label}
              accessibilityState={{ selected: chosen, disabled: busy || library.isPending }}
              disabled={busy || library.isPending}
              onPress={() => chooseFace(emoji)}
              className={`h-12 w-12 items-center justify-center overflow-hidden rounded-full border-2 bg-surface-2 ${chosen ? "border-brand" : "border-line"}`}
            >
              {imageSource ? (
                <>
                  <Image
                    source={{ uri: imageSource }}
                    contentFit="cover"
                    style={{ width: 48, height: 48 }}
                  />
                  <Text className="absolute right-0 bottom-0 text-sm">{realmojiGlyph(emoji)}</Text>
                </>
              ) : (
                <Text className="text-xl">{realmojiGlyph(emoji)}</Text>
              )}
            </Pressable>
          );
        })}
        {busy && <ActivityIndicator accessibilityLabel={t("react.sending")} />}
        <Pressable
          testID="moment-react-stamp"
          accessibilityRole="button"
          accessibilityLabel={t("react.stamp")}
          accessibilityState={{ selected: mine?.case === "stamp", disabled: busy }}
          disabled={busy}
          onPress={() => setStampPickerVisible(true)}
          className={`h-12 w-12 items-center justify-center overflow-hidden rounded-full border-2 bg-surface-2 ${mine?.case === "stamp" ? "border-brand" : "border-line"}`}
        >
          {mine?.case === "stamp" ? (
            <StampImage stamp={mine.value} size={44} />
          ) : (
            <Sticker size={22} color={colors["ink-soft"]} />
          )}
        </Pressable>
      </View>
      {moment.myReaction && (
        <Pressable
          testID="moment-react-remove"
          accessibilityRole="button"
          disabled={busy}
          onPress={removeReaction}
          className="self-start rounded-full px-3 py-2 active:bg-surface-2"
        >
          <Text className="text-sm font-bold text-ink-soft">{t("react.remove")}</Text>
        </Pressable>
      )}
      <Modal visible={taking !== null} animationType="slide" onRequestClose={() => setTaking(null)}>
        {taking !== null && (
          <CameraScreen
            key={taking}
            limitSeconds={60}
            photos
            photoOnly
            initialPosition="front"
            photoOnlyHint={t("selfie.lead", {
              face: `${realmojiGlyph(taking)} ${t(`realmoji.${FACE_KEYS[taking]}`)}`,
            })}
            onCapture={(file) => {
              const emoji = taking;
              setTaking(null);
              if (emoji !== null)
                void runForAccount(accountAtCapture, (account, signal) =>
                  uploadAndReact(account, signal, emoji, file),
                );
            }}
            onChooseImage={() => {
              const emoji = taking;
              setTaking(null);
              if (emoji !== null) void pickSelfie(emoji);
            }}
            onClose={() => setTaking(null)}
          />
        )}
      </Modal>
      <StampPicker
        visible={stampPickerVisible}
        onPick={chooseStamp}
        onClose={() => setStampPickerVisible(false)}
      />
    </View>
  );
}
