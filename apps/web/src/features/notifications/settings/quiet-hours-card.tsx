import { useMutation, useQueryClient } from "@tanstack/react-query";
import { identity, type notify } from "@tokuly/api";
import { Globe, Moon } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { TextField } from "#/components/ui/field";
import { api } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { notifyKeys } from "../api";
import { type QuietHoursValue, usePreferenceActions } from "../preferences";
import { Toggle } from "../toggle";
import { minutesToTime, timeToMinutes } from "../when";

/** Quiet hours: a nightly window without pushes, in the account's time zone. */
export function QuietHoursCard({ preferences }: { preferences: notify.Preferences }) {
  const { t } = useTranslation("notifications");
  const actions = usePreferenceActions();
  const quiet: QuietHoursValue = {
    enabled: preferences.quietHours?.enabled ?? false,
    startMinute: preferences.quietHours?.startMinute ?? 22 * 60,
    endMinute: preferences.quietHours?.endMinute ?? 7 * 60,
  };
  // What is being typed; the saved times show again once it is valid and sent.
  const [draft, setDraft] = useState<{ start?: string; end?: string }>({});
  const start = draft.start ?? minutesToTime(quiet.startMinute);
  const end = draft.end ?? minutesToTime(quiet.endMinute);
  const sameTime = timeToMinutes(start) !== null && timeToMinutes(start) === timeToMinutes(end);

  // Flipping the switch and then typing a time can both happen before the next render.
  const latest = () => actions.latestQuietHours() ?? quiet;

  const change = (edge: "start" | "end", value: string) => {
    const saved = latest();
    const next = {
      start: draft.start ?? minutesToTime(saved.startMinute),
      end: draft.end ?? minutesToTime(saved.endMinute),
      [edge]: value,
    };
    const startMinute = timeToMinutes(next.start);
    const endMinute = timeToMinutes(next.end);
    // The window needs two different, complete times; until then nothing is sent.
    if (startMinute === null || endMinute === null || startMinute === endMinute) {
      setDraft((current) => ({ ...current, [edge]: value }));
      return;
    }
    setDraft({});
    actions.setQuietHours.mutate({ enabled: saved.enabled, startMinute, endMinute });
  };

  return (
    <Card>
      <div className="flex items-center gap-3">
        <span
          className={cn(
            "sticker-sm grid size-12 shrink-0 place-items-center rounded-2xl",
            quiet.enabled ? "bg-brand text-on-brand" : "bg-surface-3 text-ink",
          )}
        >
          <Moon className="size-6" />
        </span>
        <h2 className="flex min-w-0 flex-1 flex-wrap items-center gap-2 text-lg font-black leading-snug">
          {t("quiet.title")}
          {preferences.quietNow && (
            <Chip className="bg-brand-soft text-brand-strong">{t("quiet.now")}</Chip>
          )}
        </h2>
        <Toggle
          label={t("quiet.title")}
          checked={quiet.enabled}
          onChange={(enabled) => actions.setQuietHours.mutate({ ...latest(), enabled })}
        />
      </div>
      <p className="mt-3 text-sm leading-relaxed text-ink-soft">{t("quiet.lead")}</p>

      <div
        className={cn(
          "mt-4 grid grid-cols-2 gap-3 transition-opacity",
          !quiet.enabled && "opacity-60",
        )}
      >
        <TextField
          type="time"
          label={t("quiet.start")}
          value={start}
          onChange={(event) => change("start", event.target.value)}
        />
        <TextField
          type="time"
          label={t("quiet.end")}
          value={end}
          onChange={(event) => change("end", event.target.value)}
          hint={quiet.endMinute < quiet.startMinute ? t("quiet.nextDay") : undefined}
          error={sameTime ? t("quiet.sameTime") : undefined}
        />
      </div>

      <TimeZone accountZone={preferences.timeZone} />
    </Card>
  );
}

/**
 * Quiet hours follow the account's time zone. When this device is somewhere
 * else (a trip, a move), offer to bring the account along.
 */
function TimeZone({ accountZone }: { accountZone: string }) {
  const { t } = useTranslation("notifications");
  const queryClient = useQueryClient();
  const deviceZone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const adopt = useMutation({
    mutationFn: () =>
      api().service(identity.AccountService).updateProfile({ timeZone: deviceZone }),
    onSuccess: (response) => {
      if (response.me) api().session.updateMe(response.me);
      toast.success(t("quiet.zoneChanged", { zone: deviceZone }));
      return queryClient.invalidateQueries({ queryKey: notifyKeys.preferences });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <div className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-2xl bg-surface-2 px-3 py-2.5 text-sm">
      <Globe className="size-4 shrink-0 text-ink-soft" />
      <p className="min-w-0 flex-1 font-bold">
        {t("quiet.zone", { zone: accountZone })}
        {deviceZone && deviceZone !== accountZone && (
          <span className="block font-medium text-ink-soft">
            {t("quiet.zoneDiffers", { zone: deviceZone })}
          </span>
        )}
      </p>
      {deviceZone && deviceZone !== accountZone && (
        <Button
          variant="secondary"
          size="sm"
          loading={adopt.isPending}
          onClick={() => adopt.mutate()}
        >
          {t("quiet.zoneAdopt")}
        </Button>
      )}
    </div>
  );
}
