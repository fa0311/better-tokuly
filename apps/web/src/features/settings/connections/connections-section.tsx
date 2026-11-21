import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type identity, toDate } from "@tokuly/api";
import { Music } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Skeleton } from "#/components/ui/page";
import { accountClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { keys } from "../data";
import { SettingsPage, StateChip } from "../shell/settings-page";

/** Services Tokuly can connect to; names and blurbs are `connections.providers.<id>`. */
export const PROVIDERS = ["spotify"] as const;
export type Provider = (typeof PROVIDERS)[number];

/** Where a provider sends the person back to after they agreed (or not). */
export function callbackUrl(provider: Provider): string {
  return `${window.location.origin}/settings/connections/callback/${provider}`;
}

/** Third-party services linked to the account (Spotify for "now playing"). */
export function ConnectionsSection() {
  const { t } = useTranslation("settings");
  const { data, isPending } = useQuery({ queryKey: keys.connections, queryFn: () => accountClient().listConnections({}) });

  return (
    <SettingsPage title={t("sections.connections.title")}>
      <p className="px-1 pb-4 text-sm leading-relaxed text-ink-soft">{t("connections.lead")}</p>
      <ul className="flex flex-col gap-4">
        {PROVIDERS.map((provider) => (
          <li key={provider}>
            {isPending ? (
              <Skeleton className="h-36" />
            ) : (
              <ProviderCard
                provider={provider}
                connection={data?.connections.find((connection) => connection.provider === provider)}
                available={data?.availableProviders.includes(provider) ?? false}
              />
            )}
          </li>
        ))}
      </ul>
    </SettingsPage>
  );
}

function ProviderCard({ provider, connection, available }: { provider: Provider; connection: identity.Connection | undefined; /** The server has credentials for this provider. */ available: boolean }) {
  const { t, i18n } = useTranslation("settings");
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);

  const connect = useMutation({
    mutationFn: () => accountClient().beginConnection({ provider, redirectUri: callbackUrl(provider) }),
    // Leaves the app; the provider sends the person back to the callback route.
    onSuccess: (response) => window.location.assign(response.authorizationUrl),
    onError: (error) => toast.error(errorMessage(error)),
  });
  const disconnect = useMutation({
    mutationFn: () => accountClient().deleteConnection({ provider }),
    onSuccess: () => {
      toast.success(t("connections.disconnected", { provider: t(`connections.providers.${provider}.name`) }));
      return queryClient.invalidateQueries({ queryKey: keys.connections });
    },
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: () => setConfirming(false),
  });

  const name = t(`connections.providers.${provider}.name`);
  const since = toDate(connection?.connectedAt)?.toLocaleDateString(i18n.language, { dateStyle: "medium" });

  return (
    <div className="sticker rounded-card bg-surface p-4" data-testid={`connection-${provider}`}>
      <div className="flex items-center gap-3">
        <span className="grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-mint text-[#17122b]">
          <Music className="size-6" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-lg font-black">{name}</p>
          {connection && <p className="truncate text-sm font-bold text-ink-soft">{t("connections.connectedAs", { account: connection.accountName, date: since })}</p>}
        </div>
        {connection ? <StateChip tone="good">{t("connections.connected")}</StateChip> : !available && <StateChip tone="off">{t("connections.unavailable")}</StateChip>}
      </div>
      {!connection && <p className="mt-3 text-sm leading-relaxed text-ink-soft">{t(`connections.providers.${provider}.blurb`)}</p>}

      {connection ? (
        <>
          <p className="mt-3 text-sm leading-relaxed text-ink-soft">
            {t(`connections.providers.${provider}.connectedHint`)}{" "}
            <Link to="/settings/privacy" className="font-bold text-brand underline-offset-4 hover:underline">
              {t("connections.toPrivacy")}
            </Link>
          </p>
          <Button size="sm" variant="secondary" className="mt-4" onClick={() => setConfirming(true)}>
            {t("connections.disconnect")}
          </Button>
        </>
      ) : available ? (
        <Button size="sm" className="mt-4" loading={connect.isPending} onClick={() => connect.mutate()}>
          {t("connections.connect", { provider: name })}
        </Button>
      ) : (
        <p className="mt-3 rounded-2xl bg-surface-2 px-4 py-3 text-sm leading-relaxed text-ink-soft">{t("connections.unavailableBody", { provider: name })}</p>
      )}

      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("connections.disconnectTitle", { provider: name })}
        description={t("connections.disconnectBody")}
        confirmLabel={t("connections.disconnect")}
        danger
        loading={disconnect.isPending}
        onConfirm={() => disconnect.mutate()}
      />
    </div>
  );
}
