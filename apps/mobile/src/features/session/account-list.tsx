import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { identity } from "@tokuly/api";
import { accountKeys } from "@tokuly/core/session";
import { LogOut, ShieldCheck } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { CodePrompt } from "./code-prompt";
import { removeParkedAccount, switchAccount } from "./session";

/**
 * The accounts of this device with a way into each: the active one marked,
 * the parked ones a tap away (asking for their second factor when they need
 * it), and signing a parked one out of this device.
 */
export function AccountList({ accounts }: { accounts: identity.DeviceAccount[] }) {
  const { t } = useTranslation("accounts");
  const { t: tc } = useTranslation("common");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const [asking, setAsking] = useState<identity.DeviceAccount | null>(null);

  const switchTo = useMutation({
    mutationFn: ({ account, code }: { account: identity.DeviceAccount; code?: string }) =>
      switchAccount(account.sessionId, code),
    onSuccess: (result, { account }) => setAsking(result === "needsCode" ? account : null),
    onError: (error) => {
      if (!asking) Alert.alert(errorMessage(error, { namespace: "accounts" }));
      void queryClient.invalidateQueries({ queryKey: accountKeys.deviceAccounts });
    },
  });
  const remove = useMutation({
    mutationFn: (account: identity.DeviceAccount) => removeParkedAccount(account.sessionId),
    onSettled: () => queryClient.invalidateQueries({ queryKey: accountKeys.deviceAccounts }),
    onError: (error) => Alert.alert(errorMessage(error, { namespace: "accounts" })),
  });

  const confirmRemove = (account: identity.DeviceAccount) =>
    Alert.alert(t("removeConfirm", { handle: account.user?.handle ?? "" }), undefined, [
      { text: tc("actions.cancel"), style: "cancel" },
      { text: tc("actions.ok"), style: "destructive", onPress: () => remove.mutate(account) },
    ]);

  return (
    <View>
      {accounts.map((account) => (
        <View
          key={account.sessionId}
          testID="device-account"
          className="flex-row items-center gap-3 border-b border-line px-4 py-3"
        >
          <Avatar user={account.user} size="md" />
          <View className="min-w-0 flex-1">
            <Text numberOfLines={1} className="font-black text-ink">
              {account.user?.displayName}
            </Text>
            <Text numberOfLines={1} className="text-sm font-bold text-ink-soft">
              @{account.user?.handle}
            </Text>
          </View>
          {account.active ? (
            <Text className="rounded-full bg-mint/20 px-3 py-1 text-xs font-black text-success">
              {t("active")}
            </Text>
          ) : (
            <>
              {account.mfaRequired && (
                <ShieldCheck
                  size={16}
                  color={colors["ink-soft"]}
                  accessibilityLabel={t("needsCode")}
                />
              )}
              <Button
                size="sm"
                testID={`switch-${account.user?.handle}`}
                loading={switchTo.isPending && switchTo.variables?.account === account}
                onPress={() =>
                  account.mfaRequired ? setAsking(account) : switchTo.mutate({ account })
                }
              >
                {t("switch")}
              </Button>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={t("remove", { handle: account.user?.handle ?? "" })}
                onPress={() => confirmRemove(account)}
                hitSlop={8}
                className="p-1"
              >
                <LogOut size={18} color={colors["ink-soft"]} />
              </Pressable>
            </>
          )}
        </View>
      ))}
      <CodePrompt
        handle={asking?.user?.handle ?? null}
        busy={switchTo.isPending}
        error={
          switchTo.error && asking ? errorMessage(switchTo.error, { namespace: "accounts" }) : null
        }
        onCancel={() => {
          setAsking(null);
          switchTo.reset();
        }}
        onSubmit={(code) => asking && switchTo.mutate({ account: asking, code })}
      />
    </View>
  );
}
