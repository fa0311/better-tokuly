import { useMutation } from "@tanstack/react-query";
import type { identity } from "@tokuly/api";
import { Check, Search } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { SelectField } from "#/components/ui/select";
import { accountClient } from "#/lib/api";
import { type Language, normalizeLanguage, setLanguage } from "#/lib/i18n";
import { adoptMe } from "../data";
import { errorMessage } from "#/lib/errors";
import { Group } from "../shell/settings-page";
import { allTimeZones, deviceTimeZone, searchTimeZones, zoneLabel, zoneOffset } from "./time-zone";

/** Languages are named in themselves, so the list reads the same in every language. */
const LANGUAGES: { value: Language; label: string }[] = [
  { value: "ja", label: "日本語" },
  { value: "en", label: "English" },
];

/** Most zones listed at once; typing narrows the rest down. */
const MAX_ZONES_SHOWN = 60;

/** Language of the app (and of mails from Tokuly) and the account's time zone. */
export function RegionCard({ me }: { me: identity.Me }) {
  const { t, i18n } = useTranslation("settings");
  const [pickingZone, setPickingZone] = useState(false);
  const language = normalizeLanguage(i18n.language);
  const deviceZone = deviceTimeZone();

  // Language and time zone are stored with the profile on the account.
  const save = useMutation({
    mutationFn: (patch: { locale?: string; timeZone?: string }) => accountClient().updateProfile(patch),
    onSuccess: (response, patch) => {
      adoptMe(response.me);
      if (patch.timeZone) toast.success(t("account.region.timeZoneSaved"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const changeLanguage = (next: Language) => {
    // The app switches at once; the account remembers it for mails and other devices.
    setLanguage(next);
    save.mutate({ locale: next });
  };

  return (
    <Group title={t("account.region.title")}>
      <SelectField<Language> label={t("account.region.language")} hint={t("account.region.languageHint")} value={language} onChange={changeLanguage} options={LANGUAGES} />

      <div className="mt-5">
        <p className="px-1 text-sm font-bold">{t("account.region.timeZone")}</p>
        <div className="mt-1.5 flex items-center gap-3 rounded-2xl border-2 border-line px-4 py-2.5">
          <p className="min-w-0 flex-1">
            <span className="block truncate font-bold" data-testid="account-time-zone">
              {zoneLabel(me.timeZone)}
            </span>
            <span className="block text-sm text-ink-soft">{zoneOffset(me.timeZone)}</span>
          </p>
          <Button size="sm" variant="secondary" className="shrink-0" onClick={() => setPickingZone(true)}>
            {t("account.region.changeTimeZone")}
          </Button>
        </div>
        <p className="mt-1.5 px-1 text-sm text-ink-soft">{t("account.region.timeZoneHint")}</p>
        {deviceZone !== me.timeZone && (
          <Button size="sm" variant="lime" className="mt-3" loading={save.isPending && save.variables?.timeZone === deviceZone} onClick={() => save.mutate({ timeZone: deviceZone })}>
            {t("account.region.useDeviceZone", { zone: zoneLabel(deviceZone) })}
          </Button>
        )}
      </div>

      {pickingZone && (
        <TimeZoneDialog
          current={me.timeZone}
          onClose={() => setPickingZone(false)}
          onPick={(timeZone) => {
            setPickingZone(false);
            if (timeZone !== me.timeZone) save.mutate({ timeZone });
          }}
        />
      )}
    </Group>
  );
}

function TimeZoneDialog({ current, onPick, onClose }: { current: string; onPick(zone: string): void; onClose(): void }) {
  const { t } = useTranslation("settings");
  const [query, setQuery] = useState("");
  // Before anything is typed, the zones most likely wanted lead the list: the current one and this device's.
  const likely = query.trim() ? [] : [...new Set([current, deviceTimeZone()])];
  const matches = [...likely, ...searchTimeZones(allTimeZones(), query).filter((zone) => !likely.includes(zone))];
  const shown = matches.slice(0, MAX_ZONES_SHOWN);

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t("account.region.pickTimeZone")}>
      <label className="flex h-12 items-center gap-2 rounded-control border-2 border-line bg-surface px-4 focus-within:border-brand">
        <Search className="size-5 text-ink-faint" />
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("account.region.searchTimeZone")}
          aria-label={t("account.region.searchTimeZone")}
          className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-ink-faint"
          autoCapitalize="none"
          spellCheck={false}
          autoFocus
        />
      </label>
      <ul className="mt-3 flex min-h-64 flex-col gap-0.5">
        {shown.map((zone) => (
          <li key={zone}>
            <button type="button" onClick={() => onPick(zone)} className="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left hover:bg-surface-2">
              <span className="min-w-0 flex-1 truncate font-bold">{zoneLabel(zone)}</span>
              <span className="shrink-0 text-sm tabular-nums text-ink-soft">{zoneOffset(zone)}</span>
              {zone === current && <Check className="size-5 shrink-0 text-brand" aria-label={t("account.region.currentTimeZone")} />}
            </button>
          </li>
        ))}
        {matches.length === 0 && <li className="px-3 py-8 text-center text-sm font-bold text-ink-faint">{t("account.region.noTimeZone")}</li>}
        {matches.length > shown.length && <li className="px-3 py-3 text-center text-sm text-ink-faint">{t("account.region.moreTimeZones", { count: matches.length - shown.length })}</li>}
      </ul>
    </Dialog>
  );
}
