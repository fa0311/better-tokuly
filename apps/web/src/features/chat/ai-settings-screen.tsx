import { Link, useNavigate } from "@tanstack/react-router";
import { chat } from "@tokuly/api";
import { KeyRound, Send, Trash2 } from "lucide-react";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { providerName, useAssistSettings, useAssistSettingsChange } from "./ai-settings";
import { assistClient } from "./api";
import { BotSettingsSection } from "./bot";

/**
 * `/settings/chat-ai`: which AI the assistant sends text to (among the
 * providers the operator enabled), switching it off, and API keys of one's
 * own. Who receives the text is always spelled out.
 */
export function AiSettingsScreen() {
  const { t } = useTranslation("chat");
  const navigate = useNavigate();
  const settings = useAssistSettings();
  const choose = useAssistSettingsChange(
    (input: { provider: chat.AiProvider; off: boolean }) =>
      assistClient().chooseAssistProvider(input),
    t("aiSettings.saved"),
  );
  const data = settings.data;

  return (
    <Page title={t("aiSettings.title")} back="/settings">
      {!data ? (
        <Skeleton className="h-64" />
      ) : data.providers.length === 0 ? (
        <EmptyState emoji="🤖" title={t("aiSettings.unavailable")} />
      ) : (
        <div className="flex flex-col gap-5">
          <p className="text-sm text-ink-soft">{t("aiSettings.lead")}</p>
          <p
            className="sticker flex items-center gap-2 rounded-2xl bg-sky px-4 py-3 font-black text-[#17122b]"
            data-testid="ai-recipient"
          >
            <Send className="size-4 shrink-0" />
            {data.effectiveProvider === chat.AiProvider.UNSPECIFIED
              ? t("aiSettings.nowNothing")
              : t("aiSettings.nowSending", { recipient: data.effectiveRecipient })}
          </p>

          <section className="rounded-card bg-surface p-2">
            <SwitchRow
              label={t("aiSettings.off")}
              hint={t("aiSettings.offHint")}
              checked={data.off}
              disabled={choose.isPending}
              onChange={(off) => choose.mutate({ provider: data.chosenProvider, off })}
            />
            <fieldset disabled={data.off || choose.isPending} className="disabled:opacity-50">
              <legend className="sr-only">{t("aiSettings.title")}</legend>
              <ProviderChoice
                checked={data.chosenProvider === chat.AiProvider.UNSPECIFIED}
                onChoose={() =>
                  choose.mutate({ provider: chat.AiProvider.UNSPECIFIED, off: false })
                }
                title={
                  data.providers.find((option) => option.isDefault)
                    ? t("aiSettings.followDefault", {
                        recipient: data.providers.find((option) => option.isDefault)?.recipient,
                      })
                    : t("aiSettings.followDefaultNone")
                }
              />
              {data.providers.map((option) => (
                <ProviderChoice
                  key={option.provider}
                  checked={data.chosenProvider === option.provider}
                  onChoose={() => choose.mutate({ provider: option.provider, off: false })}
                  title={providerName(option.provider)}
                  lines={[
                    t("aiSettings.recipient", { recipient: option.recipient }),
                    t("aiSettings.model", { model: option.model }),
                    ...(option.usable ? [] : [t("aiSettings.needsKey")]),
                  ]}
                />
              ))}
            </fieldset>
          </section>

          <BotSettingsSection
            settings={data}
            onOpen={(conversationId) =>
              void navigate({ to: "/chat/$conversationId", params: { conversationId } })
            }
          />

          {data.providers.some((option) => option.acceptsOwnKey) && <OwnKeys settings={data} />}

          <Link
            to="/settings/privacy"
            className="text-sm font-bold text-brand underline-offset-2 hover:underline"
          >
            {t("aiSettings.consentLink")}
          </Link>
        </div>
      )}
    </Page>
  );
}

function ProviderChoice({
  checked,
  onChoose,
  title,
  lines = [],
}: {
  checked: boolean;
  onChoose(): void;
  title: string;
  lines?: string[];
}) {
  return (
    <label
      className={cn(
        "flex cursor-pointer items-start gap-3 rounded-2xl px-3 py-3 hover:bg-surface-2",
        checked && "bg-surface-2",
      )}
    >
      <input
        type="radio"
        name="ai-provider"
        checked={checked}
        onChange={onChoose}
        className="mt-1 size-4 accent-[var(--color-brand)]"
      />
      <span className="min-w-0 flex-1">
        <span className="block font-bold">{title}</span>
        {lines.map((line) => (
          <span key={line} className="block text-sm break-words text-ink-soft">
            {line}
          </span>
        ))}
      </span>
    </label>
  );
}

function OwnKeys({ settings }: { settings: chat.AssistSettings }) {
  const { t } = useTranslation("chat");
  return (
    <section className="rounded-card bg-surface p-4">
      <h2 className="flex items-center gap-2 font-black">
        <KeyRound className="size-4" />
        {t("aiSettings.keys")}
      </h2>
      <p className="mt-1 text-sm text-ink-soft">{t("aiSettings.keysHint")}</p>
      <div className="mt-3 flex flex-col gap-4">
        {settings.providers
          .filter((option) => option.acceptsOwnKey)
          .map((option) => (
            <OwnKey
              key={option.provider}
              provider={option.provider}
              stored={settings.ownKeys.find((key) => key.provider === option.provider)}
            />
          ))}
      </div>
    </section>
  );
}

function OwnKey({ provider, stored }: { provider: chat.AiProvider; stored?: chat.OwnApiKey }) {
  const { t } = useTranslation("chat");
  const [key, setKey] = useState("");
  const save = useAssistSettingsChange(
    (apiKey: string) => assistClient().setOwnApiKey({ provider, apiKey }),
    t("aiSettings.keySaved"),
  );
  const remove = useAssistSettingsChange(
    () => assistClient().removeOwnApiKey({ provider }),
    t("aiSettings.keyRemoved"),
  );
  const name = providerName(provider);
  return (
    <form
      className="flex flex-col gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        if (key.trim()) save.mutate(key.trim(), { onSuccess: () => setKey("") });
      }}
    >
      <p className="flex flex-wrap items-center gap-2 text-sm">
        <span className="font-bold">{name}</span>
        <span className="text-ink-soft" data-testid={`own-key-${chat.AiProvider[provider]}`}>
          {stored ? t("aiSettings.keySet", { last4: stored.last4 }) : t("aiSettings.keyNone")}
        </span>
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <input
          type="password"
          autoComplete="off"
          value={key}
          maxLength={512}
          aria-label={`${name}: ${t("aiSettings.keyPlaceholder")}`}
          placeholder={t("aiSettings.keyPlaceholder")}
          onChange={(event) => setKey(event.target.value)}
          className="h-10 min-w-0 flex-1 rounded-xl border-2 border-line bg-surface px-3 font-mono text-sm outline-none focus:border-ink"
        />
        <Button type="submit" size="sm" loading={save.isPending} disabled={!key.trim()}>
          {t("aiSettings.saveKey")}
        </Button>
        {stored && (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            loading={remove.isPending}
            onClick={() => remove.mutate(undefined)}
            aria-label={`${name}: ${t("aiSettings.removeKey")}`}
          >
            <Trash2 className="size-4" />
            {t("aiSettings.removeKey")}
          </Button>
        )}
      </div>
    </form>
  );
}
