import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type common, fromDate, plans, toDate } from "@tokuly/api";
import { CalendarClock, ChevronDown, Moon, PartyPopper, Sun, Zap } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { type FreeBoard, freeClient, keys, useFriends } from "../api";
import { FriendChips } from "../friend-picker";
import { ChoiceChips } from "../parts";
import { clock, rememberAudience, rememberedAudience } from "./model";

/** A `datetime-local` value for a moment, in the device's time zone. */
function localInput(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** The device's position, or nothing when it is not available or not allowed. */
function roughPosition(): Promise<common.LatLng | undefined> {
  return new Promise((resolve) => {
    if (!navigator.geolocation) return resolve(undefined);
    navigator.geolocation.getCurrentPosition(
      (position) => resolve({ lat: position.coords.latitude, lng: position.coords.longitude } as common.LatLng),
      () => resolve(undefined),
      { maximumAge: 300_000, timeout: 5_000 },
    );
  });
}

/**
 * "ひま!" — the sheet that says so. The note and the audience are on top and
 * optional (the audience is remembered from last time); tapping a window
 * publishes the status, so the usual case is two taps: open, choose.
 */
export function SetFreeSheet({ open, onOpenChange, current }: { open: boolean; onOpenChange(open: boolean): void; current?: plans.FreeStatus }) {
  const { t, i18n } = useTranslation("plans");
  const queryClient = useQueryClient();
  const friends = useFriends();
  const [note, setNote] = useState("");
  const [audience, setAudience] = useState<plans.FreeAudience>(plans.FreeAudience.FRIENDS);
  const [chosen, setChosen] = useState<common.UserSummary[] | null>(null);
  const [rememberedIds, setRememberedIds] = useState<string[]>([]);
  const [areaName, setAreaName] = useState("");
  const [withArea, setWithArea] = useState(false);
  const [more, setMore] = useState(false);
  const [custom, setCustom] = useState(false);
  const [from, setFrom] = useState("");
  const [until, setUntil] = useState("");

  // Each opening starts from the current status, or from what was chosen last time.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      const remembered = rememberedAudience();
      setNote(current?.note ?? "");
      setAudience(current?.audience || remembered.audience);
      setChosen(current ? current.chosen : null);
      setRememberedIds(remembered.chosenIds);
      setAreaName(current?.areaName ?? "");
      setWithArea(Boolean(current?.area));
      setMore(Boolean(current?.areaName || current?.area));
      setCustom(false);
      const start = new Date(Math.ceil(Date.now() / 900_000) * 900_000);
      setFrom(localInput(start));
      setUntil(localInput(new Date(start.getTime() + 3 * 3_600_000)));
    }
  }

  // The friends chosen last time, once the friend list knows who they are.
  const chosenUsers = chosen ?? (friends.data ?? []).flatMap((friend) => (friend.user && rememberedIds.includes(friend.user.id) ? [friend.user] : []));

  const publish = useMutation({
    mutationFn: async (window: plans.FreeWindow) => {
      const area = withArea ? await roughPosition() : undefined;
      const response = await freeClient().setFreeStatus({
        window,
        startsAt: window === plans.FreeWindow.CUSTOM && from ? fromDate(new Date(from)) : undefined,
        endsAt: window === plans.FreeWindow.CUSTOM && until ? fromDate(new Date(until)) : undefined,
        note: note.trim(),
        area,
        areaName: areaName.trim(),
        audience,
        chosenUserIds: audience === plans.FreeAudience.CHOSEN ? chosenUsers.map((user) => user.id) : [],
      });
      return response.status;
    },
    onSuccess: (status) => {
      rememberAudience(audience, chosenUsers);
      queryClient.setQueryData<FreeBoard>(keys.freeFriends, (board) => (board ? { ...board, mine: status } : board));
      void queryClient.invalidateQueries({ queryKey: keys.free });
      const end = toDate(status?.endsAt);
      toast.success(t("free.published"), { description: end ? t("free.publishedUntil", { time: clock(end, i18n.language, Date.now()) }) : undefined });
      onOpenChange(false);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "plans" })),
  });

  const needsPeople = audience === plans.FreeAudience.CHOSEN && chosenUsers.length === 0;
  const pending = publish.isPending ? publish.variables : undefined;

  return (
    <Dialog open={open} onOpenChange={onOpenChange} title={current ? t("free.sheet.editTitle") : t("free.sheet.title")} description={t("free.sheet.lead")}>
      <div className="flex flex-col gap-5">
        <TextField
          label={t("free.sheet.note")}
          placeholder={t("free.sheet.notePlaceholder")}
          value={note}
          maxLength={60}
          onChange={(event) => setNote(event.target.value)}
        />

        <div className="flex flex-col gap-2">
          <p className="px-1 text-sm font-bold text-ink-soft">{t("free.sheet.audience")}</p>
          <ChoiceChips<plans.FreeAudience>
            label={t("free.sheet.audience")}
            value={audience}
            onChange={setAudience}
            options={[
              { value: plans.FreeAudience.FRIENDS, label: t("free.audience.friends") },
              { value: plans.FreeAudience.BEST_FRIENDS, label: t("free.audience.best") },
              { value: plans.FreeAudience.CHOSEN, label: t("free.audience.chosen") },
            ]}
          />
          {audience === plans.FreeAudience.CHOSEN && (
            <FriendChips users={chosenUsers} onChange={setChosen} addLabel={t("free.sheet.choose")} pickerTitle={t("free.sheet.chooseTitle")} />
          )}
        </div>

        <div>
          <button type="button" aria-expanded={more} onClick={() => setMore(!more)} className="flex items-center gap-1 px-1 text-sm font-bold text-ink-soft hover:text-ink">
            {t("free.sheet.place")}
            <ChevronDown className={cn("size-4 transition-transform", more && "rotate-180")} />
          </button>
          {more && (
            <div className="mt-2 flex flex-col gap-1">
              <TextField
                aria-label={t("free.sheet.areaName")}
                placeholder={t("free.sheet.areaPlaceholder")}
                value={areaName}
                maxLength={40}
                onChange={(event) => setAreaName(event.target.value)}
              />
              <SwitchRow label={t("free.sheet.area")} hint={t("free.sheet.areaHint")} checked={withArea} onChange={setWithArea} />
            </div>
          )}
        </div>

        <div className="flex flex-col gap-2">
          <p className="px-1 text-sm font-bold text-ink-soft">{t("free.sheet.when")}</p>
          <div className="grid grid-cols-2 gap-2.5">
            <WindowButton tint="bg-lime" icon={<Zap />} label={t("free.window.now")} hint={t("free.windowHint.now")} loading={pending === plans.FreeWindow.NOW} disabled={publish.isPending || needsPeople} onClick={() => publish.mutate(plans.FreeWindow.NOW)} />
            <WindowButton tint="bg-sun" icon={<Sun />} label={t("free.window.today")} hint={t("free.windowHint.today")} loading={pending === plans.FreeWindow.TODAY} disabled={publish.isPending || needsPeople} onClick={() => publish.mutate(plans.FreeWindow.TODAY)} />
            <WindowButton tint="bg-sky" icon={<Moon />} label={t("free.window.tonight")} hint={t("free.windowHint.tonight")} loading={pending === plans.FreeWindow.TONIGHT} disabled={publish.isPending || needsPeople} onClick={() => publish.mutate(plans.FreeWindow.TONIGHT)} />
            <WindowButton tint="bg-pink" icon={<PartyPopper />} label={t("free.window.weekend")} hint={t("free.windowHint.weekend")} loading={pending === plans.FreeWindow.WEEKEND} disabled={publish.isPending || needsPeople} onClick={() => publish.mutate(plans.FreeWindow.WEEKEND)} />
          </div>
          {needsPeople && <p className="px-1 text-sm font-bold text-danger">{t("free.sheet.needPeople")}</p>}

          <button type="button" aria-expanded={custom} onClick={() => setCustom(!custom)} className="mt-1 flex items-center gap-1.5 self-start px-1 text-sm font-bold text-ink-soft hover:text-ink">
            <CalendarClock className="size-4" />
            {t("free.window.custom")}
            <ChevronDown className={cn("size-4 transition-transform", custom && "rotate-180")} />
          </button>
          {custom && (
            <div className="flex flex-col gap-3 rounded-card bg-surface-2 p-3">
              <div className="grid gap-3 sm:grid-cols-2">
                <TextField type="datetime-local" label={t("free.sheet.from")} value={from} onChange={(event) => setFrom(event.target.value)} />
                <TextField type="datetime-local" label={t("free.sheet.until")} value={until} onChange={(event) => setUntil(event.target.value)} />
              </div>
              <Button variant="lime" loading={pending === plans.FreeWindow.CUSTOM} disabled={publish.isPending || needsPeople || !until} onClick={() => publish.mutate(plans.FreeWindow.CUSTOM)}>
                {t("free.sheet.publishCustom")}
              </Button>
            </div>
          )}
          <p className="px-1 text-xs font-bold text-ink-faint">⏳ {t("free.sheet.expires")}</p>
        </div>
      </div>
    </Dialog>
  );
}

function WindowButton({ tint, icon, label, hint, loading, disabled, onClick }: { tint: string; icon: ReactNode; label: string; hint: string; loading: boolean; disabled: boolean; onClick(): void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-busy={loading || undefined}
      className={cn("pressable sticker flex min-h-20 flex-col items-start justify-center gap-0.5 rounded-card px-4 py-3 text-left text-[#17122b] disabled:opacity-50", tint, loading && "animate-pulse")}
    >
      <span className="flex items-center gap-1.5 text-lg font-black [&>svg]:size-5">
        {icon}
        {label}
      </span>
      <span className="text-xs font-bold opacity-75">{hint}</span>
    </button>
  );
}
