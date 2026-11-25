import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { identity } from "@tokuly/api";
import { accountKeys, canAddAccount, deviceAccountsQuery } from "@tokuly/core/session";
import { Check, LogOut, Plus, ShieldCheck } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { authClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { announceAccountChange, rememberSession } from "#/stores/session";
import { Group, SettingsPage, StateChip } from "../shell/settings-page";

/**
 * Opens the app as the account that just became active. Everything cached
 * belongs to the previous one, so the page starts over from the refresh
 * cookie (which now holds the new account).
 */
export function enterAccount(): void {
  rememberSession();
  announceAccountChange();
  window.location.assign("/map");
}

/**
 * The accounts on this browser (a main one, a 裏垢, a creator account) and
 * switching between them. Each keeps its own sign-in: removing one here signs
 * only that one out of this browser, and none of them can see the others.
 */
export function AccountsSection() {
  const { t } = useTranslation("accounts");
  const queryClient = useQueryClient();
  const { data, isPending } = useQuery(deviceAccountsQuery(authClient));
  const [asking, setAsking] = useState<identity.DeviceAccount | null>(null);
  const [code, setCode] = useState("");

  const switchTo = useMutation({
    mutationFn: ({ sessionId, totpCode }: { sessionId: string; totpCode?: string }) =>
      authClient().switchAccount({ sessionId, totpCode: totpCode ?? "" }),
    onSuccess: (response, { sessionId }) => {
      if (response.mfaRequired) {
        setAsking(data?.accounts.find((a) => a.sessionId === sessionId) ?? null);
        return;
      }
      toast.success(t("switched", { name: response.me?.user?.displayName ?? "" }));
      enterAccount();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "accounts" })),
  });
  const remove = useMutation({
    mutationFn: (sessionId: string) => authClient().removeDeviceAccount({ sessionId }),
    onSuccess: () => {
      toast.success(t("removed"));
      return queryClient.invalidateQueries({ queryKey: accountKeys.deviceAccounts });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "accounts" })),
  });

  const accounts = data?.accounts ?? [];
  const full = !canAddAccount(data);
  const confirmCode = (event: FormEvent) => {
    event.preventDefault();
    if (asking) switchTo.mutate({ sessionId: asking.sessionId, totpCode: code });
  };

  return (
    <SettingsPage title={t("title")}>
      <Group title={t("listTitle")} lead={t("lead")} flush>
        {isPending ? (
          <Skeleton className="h-20" />
        ) : (
          <ul className="flex flex-col" aria-label={t("listTitle")}>
            {accounts.map((account) => (
              <li
                key={account.sessionId}
                className="flex items-center gap-3 rounded-2xl px-3 py-3"
                data-testid="device-account"
              >
                <Avatar user={account.user} size="md" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-black">
                    <EmojiText text={account.user?.displayName ?? ""} />
                  </span>
                  <span className="block truncate text-sm font-bold text-ink-soft">
                    @{account.user?.handle}
                  </span>
                </span>
                {account.active ? (
                  <StateChip tone="good" icon={<Check />}>
                    {t("active")}
                  </StateChip>
                ) : (
                  <>
                    {account.mfaRequired && (
                      <ShieldCheck className="size-4 text-ink-soft" aria-label={t("needsCode")} />
                    )}
                    <Button
                      size="sm"
                      loading={
                        switchTo.isPending && switchTo.variables?.sessionId === account.sessionId
                      }
                      onClick={() => switchTo.mutate({ sessionId: account.sessionId })}
                    >
                      {t("switch")}
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label={t("remove", { handle: account.user?.handle ?? "" })}
                      onClick={() =>
                        window.confirm(
                          t("removeConfirm", { handle: account.user?.handle ?? "" }),
                        ) && remove.mutate(account.sessionId)
                      }
                    >
                      <LogOut className="size-4" />
                    </Button>
                  </>
                )}
              </li>
            ))}
          </ul>
        )}
      </Group>

      {full ? (
        <p className="mt-5 px-1 text-sm font-bold text-ink-soft">
          {t("full", { count: data?.limit ?? 0 })}
        </p>
      ) : (
        <div className="mt-5 flex flex-wrap gap-2">
          <Link
            to="/login"
            search={{ add: true }}
            className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
          >
            <Plus className="size-4" />
            {t("add")}
          </Link>
          <Link
            to="/signup"
            search={{ add: true }}
            className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 font-bold"
          >
            {t("create")}
          </Link>
        </div>
      )}
      <p className="mt-4 px-1 text-sm leading-relaxed text-ink-soft">{t("privacyNote")}</p>

      <Dialog
        open={asking !== null}
        onOpenChange={(open) => {
          if (!open) {
            setAsking(null);
            setCode("");
          }
        }}
        title={t("codeTitle", { handle: asking?.user?.handle ?? "" })}
        description={t("codeLead")}
      >
        <form onSubmit={confirmCode} className="flex flex-col gap-4">
          <TextField
            label={t("code")}
            value={code}
            onChange={(e) => setCode(e.target.value)}
            inputMode="numeric"
            autoComplete="one-time-code"
            autoFocus
            required
          />
          <Button type="submit" block loading={switchTo.isPending}>
            {t("codeSubmit")}
          </Button>
        </form>
      </Dialog>
    </SettingsPage>
  );
}
