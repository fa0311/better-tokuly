import { useQueryClient } from "@tanstack/react-query";
import { chat, create } from "@tokuly/api";
import { Image } from "expo-image";
import * as ImagePicker from "expo-image-picker";
import { ImagePlus, SendHorizontal, X } from "lucide-react-native";
import { type ComponentRef, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, TextInput, View } from "react-native";
import { ActionSheet } from "#/components/action-sheet";
import { errorMessage } from "#/lib/errors";
import { realtime } from "#/lib/realtime";
import { useTheme } from "#/lib/theme";
import { editMessage, useCapabilities } from "./data";
import { type OutgoingPhoto, send } from "./outbox";
import { displayName, replyText } from "./text";

/** A typing indicator is repeated this often while typing goes on (others drop it after 6 s). */
const TYPING_EVERY_MS = 3_000;

function photoOf(asset: ImagePicker.ImagePickerAsset): OutgoingPhoto {
  const type = asset.mimeType ?? "image/jpeg";
  return {
    uri: asset.uri,
    name: asset.fileName ?? `photo.${type === "image/png" ? "png" : "jpg"}`,
    type,
    width: asset.width,
    height: asset.height,
  };
}

/**
 * Writing in a conversation: text, photos from the library or the camera
 * (sent through the media upload flow), a reply to a message, or editing
 * one of the viewer's own messages. Others see "typing" while it goes on.
 */
export function Composer({
  conversationId,
  myId,
  replyTo,
  editing,
  onDone,
  textOnly,
}: {
  conversationId: string;
  myId: string;
  replyTo: chat.Message | null;
  editing: chat.Message | null;
  /** Clears the reply or the edit. */
  onDone(): void;
  /** A message request still waiting: text only. */
  textOnly: boolean;
}) {
  const { t } = useTranslation("chat");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const { maxTextLength, maxAttachments } = useCapabilities();
  const [text, setText] = useState("");
  const [photos, setPhotos] = useState<OutgoingPhoto[]>([]);
  const [picking, setPicking] = useState(false);
  const typedAt = useRef(0);
  const input = useRef<ComponentRef<typeof TextInput>>(null);

  // Starting or ending an edit resets the field.
  useEffect(() => {
    setText(editing ? editing.text : "");
    if (editing) input.current?.focus();
  }, [editing]);
  useEffect(() => {
    if (replyTo) input.current?.focus();
  }, [replyTo]);

  const stopTyping = () => {
    if (typedAt.current === 0) return;
    typedAt.current = 0;
    realtime().sendTyping(conversationId, false);
  };
  // biome-ignore lint/correctness/useExhaustiveDependencies: only on leaving the conversation.
  useEffect(() => stopTyping, []);

  const change = (value: string) => {
    setText(value);
    if (editing) return;
    if (!value.trim()) stopTyping();
    else if (Date.now() - typedAt.current > TYPING_EVERY_MS) {
      typedAt.current = Date.now();
      realtime().sendTyping(conversationId, true);
    }
  };

  const pick = async (source: "library" | "camera") => {
    const options: ImagePicker.ImagePickerOptions = {
      mediaTypes: ["images"],
      quality: 0.85,
      exif: false,
      // iPhone photos (HEIC) come as JPEG: the server decodes it and strips what it carries.
      preferredAssetRepresentationMode:
        ImagePicker.UIImagePickerPreferredAssetRepresentationMode.Compatible,
    };
    if (source === "camera") {
      const permission = await ImagePicker.requestCameraPermissionsAsync();
      if (!permission.granted) return Alert.alert(tm("chat.cameraDenied"));
      const result = await ImagePicker.launchCameraAsync(options);
      if (!result.canceled) setPhotos((held) => [...held, ...result.assets.map(photoOf)]);
      return;
    }
    const permission = await ImagePicker.requestMediaLibraryPermissionsAsync();
    if (!permission.granted) return Alert.alert(tm("chat.photoDenied"));
    const result = await ImagePicker.launchImageLibraryAsync({
      ...options,
      allowsMultipleSelection: true,
      selectionLimit: Math.max(1, maxAttachments - photos.length),
    });
    if (!result.canceled)
      setPhotos((held) => [...held, ...result.assets.map(photoOf)].slice(0, maxAttachments));
  };

  const trimmed = text.trim();
  const canSend = editing ? trimmed.length > 0 : trimmed.length > 0 || photos.length > 0;
  const submit = () => {
    if (!canSend) return;
    if (editing) {
      const id = editing.id;
      editMessage(queryClient, id, trimmed).catch((error) =>
        Alert.alert(errorMessage(error, { namespace: "chat" })),
      );
    } else {
      send(queryClient, conversationId, {
        text: trimmed,
        photos,
        replyTo: replyTo ?? undefined,
      });
      typedAt.current = 0;
    }
    setText("");
    setPhotos([]);
    onDone();
  };

  const banner = editing
    ? { title: t("composer.editing"), body: editing.text }
    : replyTo
      ? {
          title: t("composer.replyingTo", { name: displayName(replyTo.sender) }),
          body: replyText(
            create(chat.ReplyReferenceSchema, {
              messageId: replyTo.id,
              seq: replyTo.seq,
              sender: replyTo.sender,
              kind: replyTo.kind,
              text: replyTo.text,
            }),
            myId,
          ),
        }
      : null;

  return (
    <View className="gap-2 border-t border-line bg-surface px-3 pt-2 pb-2">
      {banner && (
        <View className="flex-row items-center gap-2 rounded-2xl bg-surface-2 px-3 py-2">
          <View className="min-w-0 flex-1">
            <Text className="text-xs font-black text-brand">{banner.title}</Text>
            <Text numberOfLines={1} className="text-sm text-ink-soft">
              {banner.body}
            </Text>
          </View>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t("actions.cancel")}
            onPress={() => {
              if (editing) setText("");
              onDone();
            }}
          >
            <X size={18} color={colors["ink-soft"]} />
          </Pressable>
        </View>
      )}
      {photos.length > 0 && (
        <View className="flex-row flex-wrap gap-2">
          {photos.map((photo, index) => (
            <View key={photo.uri}>
              <Image source={photo.uri} style={{ width: 64, height: 64, borderRadius: 12 }} />
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={t("composer.removeFile", { name: photo.name })}
                onPress={() => setPhotos((held) => held.filter((_, at) => at !== index))}
                className="absolute -top-1.5 -right-1.5 rounded-full bg-ink p-1"
              >
                <X size={12} color={colors.surface} />
              </Pressable>
            </View>
          ))}
        </View>
      )}
      <View className="flex-row items-end gap-2">
        {!editing && !textOnly && (
          <Pressable
            testID="composer-photo"
            accessibilityRole="button"
            accessibilityLabel={t("composer.photo")}
            onPress={() => setPicking(true)}
            className="h-11 w-11 items-center justify-center rounded-full bg-surface-2"
          >
            <ImagePlus size={22} color={colors.ink} />
          </Pressable>
        )}
        <TextInput
          ref={input}
          testID="composer-input"
          accessibilityLabel={editing ? t("composer.editPlaceholder") : t("composer.placeholder")}
          placeholder={editing ? t("composer.editPlaceholder") : t("composer.placeholder")}
          placeholderTextColor={colors["ink-faint"]}
          value={text}
          onChangeText={change}
          onBlur={stopTyping}
          multiline
          maxLength={maxTextLength}
          className="max-h-32 min-h-11 flex-1 rounded-3xl bg-surface-2 px-4 py-2.5 text-base text-ink"
        />
        <Pressable
          testID="composer-send"
          accessibilityRole="button"
          accessibilityLabel={editing ? t("composer.save") : t("composer.send")}
          accessibilityState={{ disabled: !canSend }}
          disabled={!canSend}
          onPress={submit}
          className={`h-11 w-11 items-center justify-center rounded-full ${canSend ? "bg-brand" : "bg-surface-3"}`}
        >
          <SendHorizontal size={20} color={canSend ? colors["on-brand"] : colors["ink-faint"]} />
        </Pressable>
      </View>
      <ActionSheet
        visible={picking}
        onClose={() => setPicking(false)}
        actions={[
          { key: "library", label: tm("chat.photoLibrary"), onPress: () => void pick("library") },
          { key: "camera", label: tm("chat.camera"), onPress: () => void pick("camera") },
        ]}
      />
    </View>
  );
}
