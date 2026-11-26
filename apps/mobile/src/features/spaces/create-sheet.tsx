import { useQueryClient } from "@tanstack/react-query";
import { spaces } from "@tokuly/api";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { KeyboardAvoidingView, Modal, Pressable, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { TextField } from "#/components/text-field";
import { webScreenHref } from "#/features/web/handoff";
import { sticker, useTheme } from "#/lib/theme";
import { createSpaceNow } from "./session";

const CHOICES = [
  { audience: spaces.SpaceAudience.FRIENDS, key: "friends" },
  { audience: spaces.SpaceAudience.PUBLIC, key: "public" },
] as const;

/**
 * Opens a room on the spot: what it is about and who may come in. Planning
 * one ahead, tags, the language and inviting people are on the web screen.
 */
export function CreateSheet({ visible, onClose }: { visible: boolean; onClose(): void }) {
  const { t } = useTranslation("spaces");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const queryClient = useQueryClient();
  const [title, setTitle] = useState("");
  const [audience, setAudience] = useState<spaces.SpaceAudience>(spaces.SpaceAudience.FRIENDS);
  const [busy, setBusy] = useState(false);

  const start = async () => {
    setBusy(true);
    const space = await createSpaceNow(queryClient, title.trim(), audience);
    setBusy(false);
    if (!space) return;
    setTitle("");
    onClose();
    router.push(`/spaces/${space.id}`);
  };

  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      {/* Android draws edge to edge: the window is not resized for the keyboard, so both pad. */}
      <KeyboardAvoidingView behavior="padding">
        <View
          testID="spaces-create-sheet"
          className="gap-4 rounded-t-3xl bg-surface px-5 pt-5"
          style={{ paddingBottom: insets.bottom + 16 }}
        >
          <Text className="text-xl font-black text-ink">{t("form.createTitle")}</Text>
          <TextField
            testID="spaces-create-title"
            label={t("form.title")}
            placeholder={t("form.titlePlaceholder")}
            value={title}
            onChangeText={setTitle}
            maxLength={80}
          />
          <View className="gap-2">
            <Text className="text-sm font-black text-ink">{t("form.audience")}</Text>
            <View className="flex-row gap-2">
              {CHOICES.map((choice) => {
                const chosen = audience === choice.audience;
                return (
                  <Pressable
                    key={choice.key}
                    testID={`spaces-audience-${choice.key}`}
                    accessibilityRole="radio"
                    accessibilityState={{ selected: chosen }}
                    onPress={() => setAudience(choice.audience)}
                    className={`flex-1 rounded-2xl px-3 py-3 ${chosen ? "bg-brand-soft" : "bg-surface-2"}`}
                    style={chosen ? sticker(colors, "sm") : undefined}
                  >
                    <Text className="text-center text-sm font-black text-ink">
                      {t(`audience.${choice.key}`)}
                    </Text>
                  </Pressable>
                );
              })}
            </View>
            <Text className="text-xs text-ink-soft">
              {t(
                `form.audienceHint.${audience === spaces.SpaceAudience.PUBLIC ? "public" : "friends"}`,
              )}
            </Text>
          </View>
          <Button
            testID="spaces-create-start"
            size="lg"
            block
            loading={busy}
            disabled={title.trim() === ""}
            onPress={() => void start()}
          >
            {t("form.start")}
          </Button>
          <Button
            variant="ghost"
            block
            onPress={() => {
              onClose();
              router.push(webScreenHref("/spaces?tab=mine") as never);
            }}
          >
            {tm("spaces.moreOnWeb")}
          </Button>
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}
