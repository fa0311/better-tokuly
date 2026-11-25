import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { quest } from "@tokuly/api";
import { profileKeys } from "@tokuly/core/profile";
import { Check, GripVertical, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Sortable } from "#/components/ui/sortable";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { badgeClient, questKeys, useLocalized, useUserBadges } from "./api";
import { SHOWCASE_SIZE } from "./badge-screen";
import { BadgeMedal, SectionTitle, TitlePill, useBadgeName } from "./parts";

/**
 * `/quest/showcase`: which badges (up to five, in which order) sit on the
 * profile, and which title (称号) is worn next to the name. Every change is
 * saved as it is made.
 */
export function ShowcaseScreen() {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const badgeName = useBadgeName();
  const me = useMe();
  const queryClient = useQueryClient();
  const { data, isPending, isError } = useUserBadges("");

  const afterChange = () => {
    void queryClient.invalidateQueries({ queryKey: questKeys.progress });
    void queryClient.invalidateQueries({ queryKey: questKeys.profiles });
    void queryClient.invalidateQueries({ queryKey: profileKeys.profiles });
  };
  const onError = (error: unknown) => {
    toast.error(errorMessage(error, { namespace: "quest" }));
    void queryClient.invalidateQueries({ queryKey: questKeys.userBadges("") });
  };
  const patch = (change: Partial<quest.ListUserBadgesResponse>) =>
    queryClient.setQueryData<quest.ListUserBadgesResponse | undefined>(
      questKeys.userBadges(""),
      (known) => (known ? { ...known, ...change } : known),
    );

  const setShowcase = useMutation({
    mutationFn: (badges: quest.Badge[]) =>
      badgeClient().setShowcase({ badgeKeys: badges.map((badge) => badge.key) }),
    // The row moves under the finger: show the new order before the server answers.
    onMutate: (badges) => void patch({ showcase: badges }),
    onSuccess: (response) => {
      patch({ showcase: response.showcase });
      afterChange();
    },
    onError,
  });
  const setTitle = useMutation({
    mutationFn: (badgeKey: string) => badgeClient().setTitle({ badgeKey }),
    onSuccess: (response) => {
      patch({ title: response.title });
      afterChange();
      toast.success(t(response.title ? "showcase.titleSaved" : "showcase.titleRemoved"));
    },
    onError,
  });

  if (isPending)
    return (
      <Page title={t("showcase.title")} back="/quest/badges">
        <Skeleton className="h-40" />
        <Skeleton className="mt-4 h-56" />
      </Page>
    );
  if (isError || !data)
    return (
      <Page title={t("showcase.title")} back="/quest/badges">
        <EmptyState emoji="🛰️" title={t("badges.error")} />
      </Page>
    );

  const showcase = data.showcase;
  const shownKeys = new Set(showcase.map((badge) => badge.key));
  const full = showcase.length >= SHOWCASE_SIZE;
  const titled = data.badges.filter((badge) => text(badge.title));
  const wearing = data.title?.badgeKey ?? "";
  const toggle = (badge: quest.Badge) =>
    setShowcase.mutate(
      shownKeys.has(badge.key)
        ? showcase.filter((item) => item.key !== badge.key)
        : [...showcase, badge],
    );

  return (
    <Page
      title={t("showcase.title")}
      back="/quest/badges"
      actions={
        me?.user && (
          <Link
            to="/u/$handle"
            params={{ handle: me.user.handle }}
            className="text-sm font-black text-brand"
          >
            {t("showcase.viewProfile")}
          </Link>
        )
      }
    >
      <section aria-label={t("showcase.shown")}>
        <SectionTitle aside={t("showcase.slots", { count: showcase.length, max: SHOWCASE_SIZE })}>
          {t("showcase.shown")}
        </SectionTitle>
        <p className="mt-1 text-sm text-ink-soft">{t("showcase.shownHint")}</p>
        {showcase.length === 0 ? (
          <div className="mt-3 rounded-card border-2 border-dashed border-line px-4 py-8 text-center text-sm font-bold text-ink-soft">
            {t("showcase.empty")}
          </div>
        ) : (
          <Sortable
            items={showcase}
            getKey={(badge) => badge.key}
            onReorder={(badges) => setShowcase.mutate(badges)}
            className="mt-3 flex flex-col gap-2"
          >
            {(badge, { grip, dragging }) => (
              <div
                className={cn(
                  "sticker-sm flex items-center gap-3 rounded-2xl bg-surface py-2 pr-2 pl-1",
                  dragging && "bg-brand-soft",
                )}
              >
                <button
                  type="button"
                  aria-label={t("showcase.move", { name: badgeName(badge) })}
                  className="grid h-10 w-8 shrink-0 place-items-center rounded-xl text-ink-faint hover:bg-surface-2"
                  {...grip}
                >
                  <GripVertical className="size-5" />
                </button>
                <BadgeMedal badge={badge} owned size="sm" />
                <span className="min-w-0 flex-1 truncate font-bold">{badgeName(badge)}</span>
                <button
                  type="button"
                  aria-label={t("showcase.remove", { name: badgeName(badge) })}
                  onClick={() => toggle(badge)}
                  className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
                >
                  <X className="size-5" />
                </button>
              </div>
            )}
          </Sortable>
        )}
      </section>

      <section aria-label={t("showcase.owned")} className="mt-7">
        <SectionTitle>{t("showcase.owned")}</SectionTitle>
        {data.badges.length === 0 ? (
          <EmptyState emoji="🥚" title={t("showcase.noBadges")} body={t("showcase.noBadgesBody")} />
        ) : (
          <>
            {full && (
              <p className="mt-1 text-sm font-bold text-ink-soft">
                {t("badge.showcaseFull", { count: SHOWCASE_SIZE })}
              </p>
            )}
            <ul className="mt-3 grid grid-cols-3 gap-2 sm:grid-cols-4">
              {data.badges.map((badge) => {
                const shown = shownKeys.has(badge.key);
                return (
                  <li key={badge.key}>
                    <button
                      type="button"
                      aria-pressed={shown}
                      aria-label={badgeName(badge)}
                      disabled={!shown && full}
                      onClick={() => toggle(badge)}
                      className={cn(
                        "pressable relative flex h-full w-full flex-col items-center gap-2 rounded-2xl border-2 px-1 pt-3 pb-2 disabled:opacity-40",
                        shown ? "border-ink bg-brand-soft" : "border-line bg-surface",
                      )}
                    >
                      <BadgeMedal badge={badge} owned size="md" />
                      <span className="line-clamp-2 text-xs leading-tight font-bold">
                        {text(badge.name)}
                      </span>
                      {shown && (
                        <span className="absolute top-1.5 right-1.5 grid size-5 place-items-center rounded-full border-2 border-ink bg-lime text-[#17122b]">
                          <Check className="size-3" strokeWidth={4} />
                        </span>
                      )}
                    </button>
                  </li>
                );
              })}
            </ul>
          </>
        )}
      </section>

      <section aria-label={t("showcase.titles")} className="mt-7">
        <SectionTitle>{t("showcase.titles")}</SectionTitle>
        <p className="mt-1 text-sm text-ink-soft">{t("showcase.titlesHint")}</p>
        <div
          role="radiogroup"
          aria-label={t("showcase.titles")}
          className="sticker mt-3 flex flex-col rounded-card bg-surface p-2"
        >
          <TitleOption
            label={t("showcase.noTitle")}
            checked={wearing === ""}
            disabled={setTitle.isPending}
            onSelect={() => wearing !== "" && setTitle.mutate("")}
          />
          {titled.map((badge) => (
            <TitleOption
              key={badge.key}
              label={text(badge.title)}
              from={t("showcase.titleFrom", { emoji: badge.emoji, name: badgeName(badge) })}
              pill
              checked={wearing === badge.key}
              disabled={setTitle.isPending}
              onSelect={() => wearing !== badge.key && setTitle.mutate(badge.key)}
            />
          ))}
        </div>
        {titled.length === 0 && (
          <p className="mt-2 text-sm font-bold text-ink-soft">{t("showcase.noTitles")}</p>
        )}
      </section>
    </Page>
  );
}

function TitleOption({
  label,
  from,
  pill,
  checked,
  disabled,
  onSelect,
}: {
  label: string;
  /** The badge the title comes with. */
  from?: string;
  pill?: boolean;
  checked: boolean;
  disabled: boolean;
  onSelect(): void;
}) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: a row with a title pill and a note, selected like a radio.
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={onSelect}
      className={cn(
        "flex items-center gap-3 rounded-2xl px-3 py-2.5 text-left",
        checked ? "bg-brand-soft" : "hover:bg-surface-2",
      )}
    >
      <span
        className={cn(
          "grid size-5 shrink-0 place-items-center rounded-full border-2 border-ink",
          checked ? "bg-brand" : "bg-surface",
        )}
        aria-hidden
      >
        {checked && <span className="size-2 rounded-full bg-on-brand" />}
      </span>
      <span className="min-w-0 flex-1">
        {pill ? (
          <TitlePill title={label} className="h-6 px-2 text-xs" />
        ) : (
          <span className="font-bold">{label}</span>
        )}
        {from && <span className="block truncate text-xs font-bold text-ink-faint">{from}</span>}
      </span>
    </button>
  );
}
