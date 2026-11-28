import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect } from "react";
import { toast } from "sonner";
import { EmptyState } from "#/components/ui/page";
import { Spinner } from "#/components/ui/spinner";
import { accountClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { keys } from "../data";
import { SettingsPage } from "../shell/settings-page";
import { PROVIDERS, type Provider } from "./connections-section";

/**
 * `/settings/connections/callback/:provider`: where a provider (Spotify)
 * sends the person back with `code` and `state`, or with `error` when they
 * declined. Finishes the connection and returns to the list.
 */
export function ConnectionCallbackScreen({
  provider,
  code,
  state,
  error,
}: {
  provider: string;
  code: string | undefined;
  state: string | undefined;
  error: string | undefined;
}) {
  const { t, i18n } = useTranslation("settings");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const known = (PROVIDERS as readonly string[]).includes(provider);
  const complete = known && Boolean(code) && Boolean(state) && !error;

  // A query, so the one-time code is spent exactly once however often this renders.
  const finish = useQuery({
    queryKey: ["identity", "connection-finish", provider, state],
    queryFn: () =>
      accountClient().finishConnection({ provider, code: code ?? "", state: state ?? "" }),
    enabled: complete,
    retry: false,
    staleTime: Number.POSITIVE_INFINITY,
  });

  const connected = finish.data?.connection;
  useEffect(() => {
    if (!connected) return;
    const t = i18n.getFixedT(null, "settings");
    toast.success(
      t("connections.connectedToast", {
        provider: t(`connections.providers.${provider as Provider}.name`),
      }),
    );
    void queryClient.invalidateQueries({ queryKey: keys.connections });
    void navigate({ to: "/settings/connections", replace: true });
  }, [connected, provider, navigate, queryClient, i18n]);

  const back = (
    <Link
      to="/settings/connections"
      className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
    >
      {t("connections.callback.back")}
    </Link>
  );

  return (
    <SettingsPage title={t("sections.connections.title")} back="/settings/connections">
      {!known ? (
        <EmptyState emoji="🤔" title={t("connections.callback.unknown")} action={back} />
      ) : !complete ? (
        <EmptyState
          emoji="🙅"
          title={t("connections.callback.cancelled")}
          body={t("connections.callback.cancelledBody")}
          action={back}
        />
      ) : finish.isError ? (
        <EmptyState
          emoji="😢"
          title={t("connections.callback.failed")}
          body={errorMessage(finish.error, { namespace: "settings" })}
          action={back}
        />
      ) : (
        <div className="flex flex-col items-center gap-3 py-14 text-center">
          <Spinner className="size-8 text-brand" />
          <p className="font-bold text-ink-soft">{t("connections.callback.working")}</p>
        </div>
      )}
    </SettingsPage>
  );
}
