import { useQueryClient } from "@tanstack/react-query";
import type { ask } from "@tokuly/api";
import { askKeys, charCount, QUESTION_MAX } from "@tokuly/core/ask";
import { router } from "expo-router";
import { Send } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, TextInput, View } from "react-native";
import { Button } from "#/components/button";
import { sticker, useTheme } from "#/lib/theme";
import { boxAs, stillActive } from "./data";
import { Counter, showAskError } from "./parts";

/**
 * Asking a question on someone's box: 300 characters, anonymous unless the
 * asker chooses to sign (where the box allows it), a plain line about what
 * the owner will and will not see, and the "sent!" moment afterwards.
 * Attaching a picture is the web's for now.
 */
export function AskForm({ box }: { box: ask.QuestionBox }) {
  const { t } = useTranslation("ask");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const [text, setText] = useState("");
  const [wantSigned, setWantSigned] = useState(false);
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState(false);
  const owner = box.owner!;
  const name = owner.displayName;

  // Anonymous by default; signed when the asker chooses it or when it is the only way in.
  const signed = box.anonymousAvailable ? box.allowSigned && wantSigned : box.allowSigned;
  const possible = box.anonymousAvailable || box.allowSigned;
  const length = charCount(text.trim());

  const send = async () => {
    const { account, client } = boxAs();
    setSending(true);
    try {
      await client.askQuestion({ user: owner.id, text: text.trim(), signed, imageAssetId: "" });
      if (!stillActive(account)) return;
      void queryClient.invalidateQueries({ queryKey: askKeys.asked });
      setText("");
      setSent(true);
    } catch (error) {
      if (stillActive(account)) showAskError(error);
    } finally {
      setSending(false);
    }
  };

  if (sent)
    return (
      <View testID="ask-sent" className="items-center gap-2 py-4">
        <Text className="text-5xl leading-[60px]">💌</Text>
        <Text className="text-lg font-black text-ink">{t("askForm.sent")}</Text>
        <Text className="text-center text-sm text-ink-soft">{t("askForm.sentBody")}</Text>
        <View className="mt-2 flex-row flex-wrap justify-center gap-2">
          <Button testID="ask-another" variant="secondary" size="sm" onPress={() => setSent(false)}>
            {t("askForm.another")}
          </Button>
          <Button
            testID="ask-see-sent"
            variant="secondary"
            size="sm"
            onPress={() => router.push("/ask/sent")}
          >
            {t("askForm.seeSent")}
          </Button>
        </View>
      </View>
    );

  if (!possible)
    return (
      <Text className="rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
        {t("askForm.unavailable", { name })}
      </Text>
    );

  return (
    <View className="gap-3">
      <View className="rounded-control border-2 border-line bg-surface px-4 pt-3 pb-2">
        <TextInput
          testID="ask-input"
          value={text}
          onChangeText={setText}
          placeholder={t("askForm.placeholder")}
          accessibilityLabel={t("askForm.placeholder")}
          placeholderTextColor={colors["ink-faint"]}
          multiline
          textAlignVertical="top"
          className="min-h-24 text-base leading-6 text-ink"
        />
        <View className="items-end">
          <Counter text={text.trim()} max={QUESTION_MAX} />
        </View>
      </View>

      {box.anonymousAvailable && box.allowSigned && (
        <View
          accessibilityRole="radiogroup"
          className="flex-row gap-1 rounded-full bg-surface-3 p-1"
        >
          {(
            [
              ["anonymous", false, "🙈"],
              ["signed", true, "🙋"],
            ] as const
          ).map(([key, value, emoji]) => {
            const active = wantSigned === value;
            return (
              <Pressable
                key={key}
                testID={`ask-${key}`}
                accessibilityRole="radio"
                accessibilityState={{ checked: active }}
                onPress={() => setWantSigned(value)}
                className={`h-9 flex-1 flex-row items-center justify-center gap-1 rounded-full ${active ? "bg-surface" : ""}`}
                style={
                  active ? sticker(colors, "sm") : { borderWidth: 2, borderColor: "transparent" }
                }
              >
                <Text className="text-sm">{emoji}</Text>
                <Text
                  numberOfLines={1}
                  className={`text-sm font-black ${active ? "text-ink" : "text-ink-soft"}`}
                >
                  {t(`askForm.${key}`)}
                </Text>
              </Pressable>
            );
          })}
        </View>
      )}

      <View testID="ask-privacy-note" className="gap-1 rounded-2xl bg-surface-2 px-4 py-3">
        {!box.anonymousAvailable && (
          <Text className="text-sm font-bold text-ink">{t("askForm.signedOnly", { name })}</Text>
        )}
        {signed ? (
          <Text className="text-sm font-bold text-ink">{t("askForm.signedNote", { name })}</Text>
        ) : (
          <>
            <Text className="text-sm font-bold text-ink">{t("askForm.anonymousNote")}</Text>
            <Text className="text-xs text-ink-soft">{t("askForm.anonymousNote2")}</Text>
          </>
        )}
      </View>

      <Button
        testID="ask-send"
        block
        size="lg"
        icon={<Send size={20} color={colors["on-brand"]} />}
        loading={sending}
        disabled={length === 0 || length > QUESTION_MAX}
        onPress={() => void send()}
      >
        {t("askForm.send")}
      </Button>
    </View>
  );
}
