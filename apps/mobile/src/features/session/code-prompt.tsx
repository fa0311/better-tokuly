import { useState } from "react";
import { useTranslation } from "react-i18next";
import { KeyboardAvoidingView, Modal, Platform, Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { ErrorNote } from "#/components/page";
import { TextField } from "#/components/text-field";

/** Asks for the second factor of the account a switch goes to. */
export function CodePrompt({
  handle,
  busy,
  error,
  onSubmit,
  onCancel,
}: {
  handle: string | null;
  busy: boolean;
  error: string | null;
  onSubmit(code: string): void;
  onCancel(): void;
}) {
  const { t } = useTranslation("accounts");
  const { t: tc } = useTranslation("common");
  const [code, setCode] = useState("");
  const close = () => {
    setCode("");
    onCancel();
  };
  return (
    <Modal transparent visible={handle !== null} animationType="fade" onRequestClose={close}>
      <KeyboardAvoidingView
        behavior={Platform.OS === "ios" ? "padding" : undefined}
        className="flex-1 justify-end"
      >
        <Pressable
          className="absolute inset-0 bg-black/40"
          onPress={close}
          accessibilityLabel={tc("actions.close")}
        />
        <View className="gap-4 rounded-t-card bg-surface px-5 pt-6 pb-10">
          <Text className="text-lg font-black text-ink">
            {t("codeTitle", { handle: handle ?? "" })}
          </Text>
          <Text className="text-sm text-ink-soft">{t("codeLead")}</Text>
          <TextField
            testID="switch-code"
            label={t("code")}
            value={code}
            onChangeText={setCode}
            keyboardType="number-pad"
            textContentType="oneTimeCode"
            autoFocus
          />
          {error && <ErrorNote>{error}</ErrorNote>}
          <Button
            block
            loading={busy}
            disabled={!code.trim()}
            onPress={() => onSubmit(code.trim())}
          >
            {t("codeSubmit")}
          </Button>
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}
