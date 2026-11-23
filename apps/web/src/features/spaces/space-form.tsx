import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { type common, fromDate, spaces, toMillis } from "@tokuly/api";
import { Plus, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { Tabs } from "#/components/ui/tabs";
import { accountTimeZone, zonedMoment } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { keys, rememberSpace, spaceClient, useSpacesConfig } from "./api";
import { FriendPicker } from "./friend-picker";
import { parseTags } from "./model";
import { AudienceLabel, PersonName } from "./parts";
import { adoptSpace, ensureAudioFocus } from "./session";
import { inputsOf, minuteOf, suggestedStart } from "./when";

const AUDIENCES = [
  spaces.SpaceAudience.FRIENDS,
  spaces.SpaceAudience.PUBLIC,
  spaces.SpaceAudience.INVITED,
] as const;

type Language = "none" | "ja" | "en";

export interface SpaceFormProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  /** The room being changed; without it the form creates one. */
  space?: spaces.Space;
  /** Opens on "schedule" rather than "start now" (creating only). */
  schedule?: boolean;
}

/**
 * Opens a room on the spot, plans one, or changes one: title, tags, who may
 * come, the language, the time and friends to invite right away.
 */
export function SpaceForm({ open, onOpenChange, space, schedule }: SpaceFormProps) {
  // The fields start from the room each time the sheet opens.
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={<FormTitle space={space} />}
      className="sm:w-[min(34rem,calc(100vw-2rem))]"
    >
      {open && <Fields space={space} schedule={schedule} onDone={() => onOpenChange(false)} />}
    </Dialog>
  );
}

function FormTitle({ space }: { space?: spaces.Space }) {
  const { t } = useTranslation("spaces");
  return space ? t("form.editTitle") : t("form.createTitle");
}

function Fields({
  space,
  schedule,
  onDone,
}: {
  space?: spaces.Space;
  schedule?: boolean;
  onDone(): void;
}) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const config = useSpacesConfig().data;
  const zone = accountTimeZone();
  const upcoming = space?.state === spaces.SpaceState.SCHEDULED;
  const planned = toMillis(space?.scheduledAt);

  const [title, setTitle] = useState(space?.title ?? "");
  const [tags, setTags] = useState(space ? space.tags.map((tag) => `#${tag}`).join(" ") : "");
  const [audience, setAudience] = useState(space?.audience ?? spaces.SpaceAudience.FRIENDS);
  const [language, setLanguage] = useState<Language>(
    space?.language === "ja" || space?.language === "en" ? space.language : "none",
  );
  const [mode, setMode] = useState<"now" | "later">(schedule || upcoming ? "later" : "now");
  const [start, setStart] = useState(() =>
    planned ? inputsOf(planned, zone) : suggestedStart(Date.now(), zone),
  );
  const [invitees, setInvitees] = useState<common.UserSummary[]>([]);
  const [picking, setPicking] = useState(false);

  const audioOff = config?.audioEnabled === false;
  const minute = minuteOf(start.clock);
  const scheduledAt =
    mode === "later" && start.date && minute !== null
      ? zonedMoment(start.date, minute, zone)
      : undefined;
  const parsedTags = parseTags(tags);
  const maxTags = config?.maxTags || 5;

  const save = useMutation({
    mutationFn: async () => {
      // Going live takes the microphone: a running call ends first, if the person agrees.
      if (!space && mode === "now" && !(await ensureAudioFocus())) return null;
      const when = scheduledAt ? fromDate(new Date(scheduledAt)) : undefined;
      const shared = {
        title: title.trim(),
        tags: parsedTags,
        language: language === "none" ? "" : language,
      };
      if (!space)
        return spaceClient().createSpace({
          ...shared,
          audience,
          scheduledAt: when,
          inviteeIds: invitees.map((user) => user.id),
        });
      return spaceClient().updateSpace({
        spaceId: space.id,
        ...shared,
        setTags: true,
        // Who may come and when can only change while the room is still ahead.
        ...(upcoming ? { audience, scheduledAt: when } : {}),
      });
    },
    onSuccess: async (response) => {
      const saved = response?.space;
      if (!saved) return;
      rememberSpace(queryClient, saved);
      void queryClient.invalidateQueries({ queryKey: keys.lists });
      onDone();
      if (space) return void toast.success(t("toast.saved"));
      if (response && "credentials" in response && saved.state === spaces.SpaceState.LIVE) {
        void navigate({ to: "/spaces/$spaceId", params: { spaceId: saved.id } });
        await adoptSpace(queryClient, saved, response.credentials);
      } else {
        toast.success(t("toast.scheduled"));
        void navigate({ to: "/spaces/$spaceId", params: { spaceId: saved.id } });
      }
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "spaces" })),
  });

  const ready =
    title.trim().length > 0 &&
    parsedTags.length <= maxTags &&
    (mode === "now" ? !audioOff || Boolean(space) : scheduledAt !== undefined);

  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={(event) => {
        event.preventDefault();
        if (ready && !save.isPending) save.mutate();
      }}
    >
      <TextField
        label={t("form.title")}
        placeholder={t("form.titlePlaceholder")}
        value={title}
        maxLength={config?.maxTitleLength || 80}
        onChange={(event) => setTitle(event.target.value)}
        autoFocus={!space}
        required
      />
      <TextField
        label={t("form.tags")}
        placeholder={t("form.tagsPlaceholder")}
        value={tags}
        onChange={(event) => setTags(event.target.value)}
        hint={t("form.tagsHint", { count: maxTags })}
        error={parsedTags.length > maxTags ? t("form.tooManyTags", { count: maxTags }) : undefined}
      />

      {(!space || upcoming) && (
        <fieldset className="flex flex-col gap-1.5">
          <legend className="mb-1.5 px-1 text-sm font-bold text-ink-soft">
            {t("form.audience")}
          </legend>
          {AUDIENCES.map((option) => {
            const chosen = audience === option;
            return (
              <label
                key={option}
                className={cn(
                  "flex cursor-pointer items-center gap-3 rounded-2xl border-2 px-4 py-3 transition-colors",
                  chosen ? "border-ink bg-brand-soft" : "border-line hover:border-ink-faint",
                )}
              >
                <input
                  type="radio"
                  name="audience"
                  className="sr-only"
                  checked={chosen}
                  onChange={() => setAudience(option)}
                />
                <span className="min-w-0 flex-1">
                  <AudienceLabel audience={option} className="text-sm text-ink" />
                  <span className="block text-xs text-ink-soft">
                    {t(
                      `form.audienceHint.${
                        option === spaces.SpaceAudience.PUBLIC
                          ? "public"
                          : option === spaces.SpaceAudience.INVITED
                            ? "invited"
                            : "friends"
                      }`,
                    )}
                  </span>
                </span>
                <span
                  className={cn(
                    "size-5 shrink-0 rounded-full border-2",
                    chosen ? "border-ink bg-lime" : "border-line",
                  )}
                />
              </label>
            );
          })}
        </fieldset>
      )}

      <SelectField<Language>
        label={t("form.language")}
        value={language}
        onChange={setLanguage}
        options={[
          { value: "none", label: t("language.none") },
          { value: "ja", label: t("language.ja") },
          { value: "en", label: t("language.en") },
        ]}
      />

      {(!space || upcoming) && (
        <div className="flex flex-col gap-3">
          {!space && (
            <Tabs<"now" | "later">
              items={[
                { value: "now", label: t("form.now") },
                { value: "later", label: t("form.later") },
              ]}
              value={mode}
              onChange={setMode}
              className="self-start"
            />
          )}
          {mode === "later" ? (
            <div className="grid grid-cols-2 gap-3">
              <TextField
                type="date"
                label={t("form.date")}
                value={start.date}
                onChange={(event) => setStart({ ...start, date: event.target.value })}
                required
              />
              <TextField
                type="time"
                label={t("form.time")}
                value={start.clock}
                onChange={(event) => setStart({ ...start, clock: event.target.value })}
                required
              />
              <p className="col-span-2 px-1 text-xs text-ink-faint">
                {t("form.laterHint", { days: config?.maxScheduleDays || 30 })}
              </p>
            </div>
          ) : (
            audioOff && (
              <p role="alert" className="px-1 text-sm font-bold text-danger">
                {t("audioOff")}
              </p>
            )
          )}
        </div>
      )}

      {!space && (
        <div className="flex flex-col gap-1.5">
          <p className="px-1 text-sm font-bold text-ink-soft">{t("form.invitees")}</p>
          <div className="flex flex-wrap items-center gap-2">
            {invitees.map((user) => (
              <span
                key={user.id}
                className="inline-flex h-10 max-w-full items-center gap-2 rounded-full border-2 border-line bg-surface pr-1 pl-1.5"
              >
                <Avatar user={user} size="xs" />
                <span className="truncate text-sm font-bold">
                  <PersonName user={user} />
                </span>
                <button
                  type="button"
                  aria-label={t("picker.remove", { name: user.displayName || user.handle })}
                  onClick={() => setInvitees(invitees.filter((other) => other.id !== user.id))}
                  className="grid size-7 shrink-0 place-items-center rounded-full text-ink-faint hover:bg-surface-2 hover:text-ink"
                >
                  <X className="size-4" />
                </button>
              </span>
            ))}
            <button
              type="button"
              onClick={() => setPicking(true)}
              className="inline-flex h-10 items-center gap-1.5 rounded-full border-2 border-dashed border-ink-faint px-4 text-sm font-bold text-ink-soft hover:border-ink hover:text-ink"
            >
              <Plus className="size-4" />
              {t("form.addInvitees")}
            </button>
          </div>
          <FriendPicker
            open={picking}
            onOpenChange={setPicking}
            title={t("form.pickInvitees")}
            initial={invitees.map((user) => user.id)}
            confirmLabel={t("picker.confirm")}
            onConfirm={(users) => {
              setInvitees(users);
              setPicking(false);
            }}
          />
        </div>
      )}

      <Button type="submit" size="lg" block loading={save.isPending} disabled={!ready}>
        {space ? t("form.save") : mode === "now" ? t("form.start") : t("form.schedule")}
      </Button>
    </form>
  );
}
