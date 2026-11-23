import { create } from "@bufbuild/protobuf";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { live } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { chatClient, patchStream } from "../api";

const { ChatAudience, LinkPolicy } = live;

/** Slow mode choices in seconds; `chatSettings.slow.<key>` names them. */
export const SLOW_MODES = [
  { seconds: 0, key: "off" },
  { seconds: 5, key: "s5" },
  { seconds: 10, key: "s10" },
  { seconds: 30, key: "s30" },
  { seconds: 60, key: "s60" },
  { seconds: 300, key: "s300" },
] as const;

const AUDIENCES = [
  { value: ChatAudience.EVERYONE, key: "everyone" },
  { value: ChatAudience.MEMBERS, key: "members" },
  { value: ChatAudience.SUBSCRIBERS, key: "subscribers" },
] as const;

/** The chat rules as a form: on or off, who writes, slow mode, links, stamps. */
export function ChatSettingsFields({
  value,
  onChange,
}: {
  value: live.ChatSettings;
  onChange(next: live.ChatSettings): void;
}) {
  const { t } = useTranslation("live");
  const set = (patch: Partial<live.ChatSettings>) => onChange({ ...value, ...patch });
  const slow = SLOW_MODES.find((mode) => mode.seconds === value.slowModeSeconds);
  return (
    <div className="flex flex-col gap-2">
      <SwitchRow
        label={t("chatSettings.enabled")}
        hint={t("chatSettings.enabledHint")}
        checked={value.enabled}
        onChange={(enabled) => set({ enabled })}
      />
      <SelectField
        label={t("chatSettings.audience")}
        value={String(value.audience || ChatAudience.EVERYONE)}
        onChange={(next) => set({ audience: Number(next) as live.ChatAudience })}
        options={AUDIENCES.map((audience) => ({
          value: String(audience.value),
          label: t(`chatSettings.audiences.${audience.key}`),
          hint: t(`chatSettings.audiences.${audience.key}Hint`),
        }))}
      />
      <SelectField
        label={t("chatSettings.slowMode")}
        hint={t("chatSettings.slowModeHint")}
        value={String(slow?.seconds ?? value.slowModeSeconds)}
        onChange={(next) => set({ slowModeSeconds: Number(next) })}
        options={[
          ...SLOW_MODES.map((mode) => ({
            value: String(mode.seconds),
            label: t(`chatSettings.slow.${mode.key}`),
          })),
          ...(slow
            ? []
            : [
                {
                  value: String(value.slowModeSeconds),
                  label: t("chatSettings.slow.custom", { count: value.slowModeSeconds }),
                },
              ]),
        ]}
      />
      <SwitchRow
        label={t("chatSettings.links")}
        hint={t("chatSettings.linksHint")}
        checked={value.linkPolicy !== LinkPolicy.BLOCK}
        onChange={(allowed) => set({ linkPolicy: allowed ? LinkPolicy.ALLOW : LinkPolicy.BLOCK })}
      />
      <SwitchRow
        label={t("chatSettings.stamps")}
        checked={value.stampsEnabled}
        onChange={(stampsEnabled) => set({ stampsEnabled })}
      />
    </div>
  );
}

/** The defaults a new stream's chat starts with. */
export function defaultChatSettings(): live.ChatSettings {
  return create(live.ChatSettingsSchema, {
    enabled: true,
    audience: ChatAudience.EVERYONE,
    linkPolicy: LinkPolicy.ALLOW,
    stampsEnabled: true,
  });
}

/** Changes the chat rules of a stream that is running. The streamer's alone. */
export function ChatSettingsDialog({
  stream,
  open,
  onOpenChange,
}: {
  stream: live.Stream;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("live");
  const queryClient = useQueryClient();
  const streamId = stream.summary?.id ?? "";
  const [draft, setDraft] = useState<live.ChatSettings | null>(null);
  const value = draft ?? stream.chat ?? defaultChatSettings();
  const save = useMutation({
    mutationFn: () => chatClient().updateChatSettings({ streamId, chat: value }),
    onSuccess: ({ chat }) => {
      if (chat) patchStream(queryClient, streamId, (current) => ({ ...current, chat }));
      toast.success(t("chatSettings.saved"));
      setDraft(null);
      onOpenChange(false);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) setDraft(null);
        onOpenChange(next);
      }}
      title={t("chatSettings.title")}
      footer={
        <Button loading={save.isPending} disabled={draft === null} onClick={() => save.mutate()}>
          {t("common:actions.save")}
        </Button>
      }
    >
      <ChatSettingsFields value={value} onChange={setDraft} />
    </Dialog>
  );
}
