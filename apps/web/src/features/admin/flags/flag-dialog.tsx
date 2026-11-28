import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { admin, common } from "@tokuly/api";
import { X } from "lucide-react";
import { useId, useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { Slider } from "#/components/ui/slider";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { flagClient } from "../api";
import { UserPicker } from "../ui/user-picker";

/** 2-64 characters of a-z, 0-9, `_`, `-`, `.`, starting with a letter. */
const NAME_PATTERN = /^[a-z][a-z0-9_.-]{1,63}$/;
const DESCRIPTION_MAX = 500;

/** A typed rollout as a whole number of percent; an emptied field counts as 0. */
const clampPercent = (value: number) =>
  Number.isFinite(value) ? Math.min(100, Math.max(0, Math.round(value))) : 0;

/** Creates a flag (`flag` unset) or edits one. Mount it fresh for every opening so it starts from the flag's current values. */
export function FlagDialog({
  flag,
  open,
  onOpenChange,
}: {
  flag?: admin.FeatureFlag;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation(["admin", "common"]);
  const queryClient = useQueryClient();
  const [name, setName] = useState(flag?.name ?? "");
  const [description, setDescription] = useState(flag?.description ?? "");
  const [enabled, setEnabled] = useState(flag?.enabled ?? false);
  const [rollout, setRollout] = useState(flag?.rolloutPercent ?? 100);
  const [allowed, setAllowed] = useState<common.UserSummary[]>(flag?.allowUsers ?? []);
  const [nameError, setNameError] = useState(false);
  const rolloutId = useId();

  const save = useMutation({
    mutationFn: async () => {
      if (!flag) {
        await flagClient().createFlag({
          name: name.trim(),
          description: description.trim(),
          enabled,
          rolloutPercent: rollout,
          allowUserIds: allowed.map((user) => user.id),
        });
        return;
      }
      const before = new Set(flag.allowUsers.map((user) => user.id));
      const after = new Set(allowed.map((user) => user.id));
      await flagClient().updateFlag({
        name: flag.name,
        description: description.trim(),
        enabled,
        rolloutPercent: rollout,
        addAllowUserIds: [...after].filter((id) => !before.has(id)),
        removeAllowUserIds: [...before].filter((id) => !after.has(id)),
      });
    },
    onSuccess: () => {
      toast.success(t(flag ? "admin:flags.toast.updated" : "admin:flags.toast.created"));
      onOpenChange(false);
      return queryClient.invalidateQueries({ queryKey: ["admin"] });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "admin" })),
  });

  const submit = () => {
    if (!flag && !NAME_PATTERN.test(name.trim())) {
      setNameError(true);
      return;
    }
    save.mutate();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={
        flag ? (
          <code className="font-mono text-lg break-all">{flag.name}</code>
        ) : (
          t("admin:flags.create.title")
        )
      }
      description={flag ? t("admin:flags.edit.description") : t("admin:flags.create.description")}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={save.isPending}>
            {t("common:actions.cancel")}
          </Button>
          <Button onClick={submit} loading={save.isPending}>
            {flag ? t("common:actions.save") : t("admin:flags.create.confirm")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        {!flag && (
          <TextField
            label={t("admin:flags.fields.name")}
            hint={t("admin:flags.fields.nameHint")}
            error={nameError ? t("errors:FLAG_NAME_INVALID") : undefined}
            value={name}
            maxLength={64}
            placeholder="new_map_pins"
            onChange={(event) => {
              setName(event.target.value);
              setNameError(false);
            }}
            autoCapitalize="none"
            spellCheck={false}
          />
        )}
        <TextArea
          label={t("admin:flags.fields.description")}
          value={description}
          maxLength={DESCRIPTION_MAX}
          onChange={(event) => setDescription(event.target.value)}
        />
        <div className="-mx-3">
          <SwitchRow
            label={t("admin:flags.fields.enabled")}
            hint={t("admin:flags.fields.enabledHint")}
            checked={enabled}
            onChange={setEnabled}
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between gap-3 px-1">
            <label htmlFor={rolloutId} className="text-sm font-bold text-ink-soft">
              {t("admin:flags.fields.rollout")}
            </label>
            <input
              id={rolloutId}
              type="number"
              inputMode="numeric"
              min={0}
              max={100}
              value={rollout}
              onChange={(event) => setRollout(clampPercent(event.target.valueAsNumber))}
              className="h-10 w-20 rounded-control border-2 border-line bg-surface px-3 text-right font-black tabular-nums focus:border-brand focus:outline-none"
            />
          </div>
          <Slider
            label={t("admin:flags.fields.rollout")}
            valueText={`${rollout}%`}
            value={rollout}
            onChange={setRollout}
          />
          <p className="px-1 text-xs text-ink-faint">{t("admin:flags.fields.rolloutHint")}</p>
        </div>
        <div className="flex flex-col gap-2">
          <p className="px-1 text-sm font-bold text-ink-soft">
            {t("admin:flags.fields.allowList", { count: allowed.length })}
          </p>
          {allowed.length > 0 && (
            <ul className="flex flex-wrap gap-1.5">
              {allowed.map((user) => (
                <li
                  key={user.id}
                  className="flex h-8 items-center gap-1.5 rounded-full bg-surface-2 pr-1 pl-1.5 text-sm font-bold"
                >
                  <Avatar user={user} size="xs" />
                  <span className="max-w-32 truncate">@{user.handle}</span>
                  <button
                    type="button"
                    aria-label={t("admin:flags.fields.allowRemove", { handle: user.handle })}
                    onClick={() =>
                      setAllowed((current) => current.filter((held) => held.id !== user.id))
                    }
                    className="grid size-6 place-items-center rounded-full hover:bg-surface-3"
                  >
                    <X className="size-3.5" />
                  </button>
                </li>
              ))}
            </ul>
          )}
          <UserPicker
            label={t("admin:flags.fields.allowAdd")}
            placeholder={t("admin:flags.fields.allowAdd")}
            excludeIds={allowed.map((user) => user.id)}
            onPick={(account) => {
              const user = account.user;
              if (user) setAllowed((current) => [...current, user]);
            }}
          />
          <p className="px-1 text-xs text-ink-faint">{t("admin:flags.fields.allowHint")}</p>
        </div>
      </div>
    </Dialog>
  );
}
