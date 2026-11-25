import { chat } from "@tokuly/api";
import { ShieldAlert } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { useStaffAccess } from "#/features/admin";
import { cn } from "#/lib/cn";
import {
  AI_PROVIDERS,
  assistAdminClient,
  providerName,
  useAssistInstance,
  useAssistInstanceChange,
} from "./ai-settings";

type DefaultName = "FIRST" | "ANTHROPIC" | "OPENAI" | "OPENAI_COMPATIBLE";

/**
 * `/admin/chat-ai`: which AI providers this server offers, with their keys,
 * models and addresses, and which one users get by default. Admin role
 * only. Saved keys are never shown again, only their last four characters.
 */
export function AiAdminScreen() {
  const { t } = useTranslation("chat");
  const access = useStaffAccess();
  const instance = useAssistInstance(access.admin);
  const setDefault = useAssistInstanceChange((provider: chat.AiProvider) =>
    assistAdminClient().setDefaultAssistProvider({ provider }),
  );
  const data = instance.data;

  return (
    <div className="h-full overflow-y-auto">
      <header className="glass sticky top-0 z-20 border-b-2 border-line">
        <div className="flex h-14 items-center px-4">
          <h1 className="min-w-0 flex-1 truncate text-lg font-black">{t("aiAdmin.title")}</h1>
        </div>
      </header>
      <div className="mx-auto flex max-w-3xl flex-col gap-4 px-4 pt-4 pb-32 md:pb-10">
        {!access.admin ? (
          <EmptyState emoji="🔐" title={t("aiAdmin.gate")} />
        ) : !data ? (
          <Skeleton className="h-96" />
        ) : (
          <>
            <p className="text-sm text-ink-soft">{t("aiAdmin.lead")}</p>
            <section className="rounded-2xl border-2 border-line bg-surface p-4">
              <SelectField<DefaultName>
                label={t("aiAdmin.defaultProvider")}
                hint={t("aiAdmin.effectiveDefault", {
                  name:
                    data.effectiveDefault === chat.AiProvider.UNSPECIFIED
                      ? t("aiAdmin.none")
                      : providerName(data.effectiveDefault),
                })}
                value={
                  data.defaultProvider === chat.AiProvider.UNSPECIFIED
                    ? "FIRST"
                    : (chat.AiProvider[data.defaultProvider] as DefaultName)
                }
                disabled={setDefault.isPending}
                onChange={(next) =>
                  setDefault.mutate(
                    next === "FIRST" ? chat.AiProvider.UNSPECIFIED : chat.AiProvider[next],
                  )
                }
                options={[
                  { value: "FIRST", label: t("aiAdmin.defaultFirst") },
                  ...AI_PROVIDERS.map((provider) => ({
                    value: chat.AiProvider[provider] as DefaultName,
                    label: providerName(provider),
                  })),
                ]}
              />
            </section>
            {data.providers.map((config) => (
              <ProviderForm
                key={`${config.provider}:${config.updatedAt?.seconds ?? "env"}`}
                config={config}
              />
            ))}
          </>
        )}
      </div>
    </div>
  );
}

function ProviderForm({ config }: { config: chat.AssistProviderConfig }) {
  const { t } = useTranslation("chat");
  const compatible = config.provider === chat.AiProvider.OPENAI_COMPATIBLE;
  const [enabled, setEnabled] = useState(config.enabled);
  const [model, setModel] = useState(config.model);
  const [baseUrl, setBaseUrl] = useState(config.baseUrl);
  const [newKey, setNewKey] = useState("");
  const [clearKey, setClearKey] = useState(false);
  const [allowPrivateHost, setAllowPrivateHost] = useState(config.allowPrivateHost);
  const save = useAssistInstanceChange(() =>
    assistAdminClient().updateAssistProvider({
      provider: config.provider,
      enabled,
      model: model.trim(),
      baseUrl: baseUrl.trim(),
      apiKeyChange: newKey.trim()
        ? chat.ApiKeyChange.SET
        : clearKey
          ? chat.ApiKeyChange.CLEAR
          : chat.ApiKeyChange.UNSPECIFIED,
      apiKey: newKey.trim(),
      allowPrivateHost: compatible && allowPrivateHost,
    }),
  );
  const name = providerName(config.provider);

  return (
    <form
      aria-label={name}
      className="rounded-2xl border-2 border-line bg-surface"
      onSubmit={(event) => {
        event.preventDefault();
        save.mutate(undefined, {
          onSuccess: () => {
            setNewKey("");
            setClearKey(false);
          },
        });
      }}
    >
      <header className="flex flex-wrap items-center gap-2 border-b-2 border-line px-4 py-2">
        <h2 className="min-w-0 flex-1 truncate font-black">{name}</h2>
        {config.fromEnvironment && (
          <span className="rounded-full bg-surface-2 px-2 py-0.5 text-xs font-bold text-ink-soft">
            {t("aiAdmin.fromEnvironment")}
          </span>
        )}
        <span
          className={cn(
            "rounded-full px-2 py-0.5 text-xs font-black",
            config.ready ? "bg-lime text-[#17122b]" : "bg-surface-3 text-ink-soft",
          )}
        >
          {config.ready ? t("aiAdmin.ready") : t("aiAdmin.notReady")}
        </span>
      </header>
      <div className="flex flex-col gap-3 p-4">
        <SwitchRow label={t("aiAdmin.enabled")} checked={enabled} onChange={setEnabled} />
        <TextField
          label={t("aiAdmin.model")}
          value={model}
          maxLength={200}
          onChange={(event) => setModel(event.target.value)}
        />
        <TextField
          label={t("aiAdmin.baseUrl")}
          hint={compatible ? "http://lmstudio.local:1234/v1" : t("aiAdmin.baseUrlHint")}
          value={baseUrl}
          inputMode="url"
          maxLength={500}
          onChange={(event) => setBaseUrl(event.target.value)}
        />
        <div className="flex flex-col gap-1.5">
          <p className="text-sm">
            <span className="font-bold">
              {compatible ? t("aiAdmin.apiKeyOptional") : t("aiAdmin.apiKey")}:{" "}
            </span>
            <span
              className="text-ink-soft"
              data-testid={`admin-key-${chat.AiProvider[config.provider]}`}
            >
              {config.apiKeySet
                ? t("aiAdmin.keySet", { last4: config.apiKeyLast4 })
                : t("aiAdmin.keyNone")}
            </span>
          </p>
          <input
            type="password"
            autoComplete="off"
            value={newKey}
            maxLength={512}
            aria-label={`${name}: ${t("aiAdmin.newKey")}`}
            placeholder={t("aiAdmin.newKey")}
            onChange={(event) => setNewKey(event.target.value)}
            className="h-11 rounded-xl border-2 border-line bg-surface px-3 font-mono text-sm outline-none focus:border-ink"
          />
          {config.apiKeySet && (
            <label className="flex items-center gap-2 text-sm font-bold">
              <input
                type="checkbox"
                checked={clearKey}
                disabled={Boolean(newKey.trim())}
                onChange={(event) => setClearKey(event.target.checked)}
              />
              {t("aiAdmin.clearKey")}
            </label>
          )}
        </div>
        {compatible && (
          <div className="rounded-2xl border-2 border-dashed border-line">
            <SwitchRow
              label={
                <span className="flex items-center gap-1.5">
                  <ShieldAlert className="size-4 text-danger" />
                  {t("aiAdmin.allowPrivate")}
                </span>
              }
              hint={t("aiAdmin.allowPrivateHint")}
              checked={allowPrivateHost}
              onChange={setAllowPrivateHost}
            />
          </div>
        )}
        <div className="flex justify-end">
          <Button type="submit" size="sm" loading={save.isPending}>
            {t("aiAdmin.save")}
          </Button>
        </div>
      </div>
    </form>
  );
}
