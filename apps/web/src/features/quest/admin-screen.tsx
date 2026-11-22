import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type common, quest, toDate } from "@tokuly/api";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { TextArea, TextField } from "#/components/ui/field";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { useStaffAccess } from "#/features/admin";
import { EmojiText } from "#/features/emoji";
import { profileClient } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { questAdminClient, questKeys, useBadgeCatalog, useLocalized } from "./api";
import { isHiddenSecret } from "./model";
import { SectionTitle, useBadgeName } from "./parts";

const REASON_LIMIT = 500;

/**
 * `/quest/admin` (admins only): giving a player a badge by hand — event
 * prizes, the special family — taking one back, and the record of both.
 * Every call needs a reason and is recorded whether or not it changed
 * anything.
 */
export function QuestAdminScreen() {
  const { t } = useTranslation("quest");
  const access = useStaffAccess();
  return (
    <Page title={t("admin.title")} back="/admin">
      {access.admin ? <AdminTools /> : <EmptyState emoji="🔒" title={t("admin.forbidden")} />}
    </Page>
  );
}

function AdminTools() {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const badgeName = useBadgeName();
  const queryClient = useQueryClient();
  const catalog = useBadgeCatalog();
  const [handle, setHandle] = useState("");
  const [player, setPlayer] = useState<common.UserSummary | null>(null);
  const [badgeKey, setBadgeKey] = useState("");
  const [reason, setReason] = useState("");

  const find = useMutation({
    mutationFn: async (value: string) =>
      (await profileClient().getProfile({ by: { case: "handle", value } })).profile?.user,
    onSuccess: (user) => {
      setPlayer(user ?? null);
      if (!user) toast.error(t("admin.userNotFound"));
    },
    onError: () => {
      setPlayer(null);
      toast.error(t("admin.userNotFound"));
    },
  });

  const afterAction = () => {
    setReason("");
    void queryClient.invalidateQueries({ queryKey: ["quest", "admin"] });
  };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "quest" }));
  const target = { userId: player?.id ?? "", badgeKey, reason: reason.trim() };
  const award = useMutation({
    mutationFn: () => questAdminClient().awardBadge(target),
    onSuccess: (response) => {
      afterAction();
      toast.success(t(response.awarded ? "admin.awarded" : "admin.alreadyOwned"));
    },
    onError,
  });
  const revoke = useMutation({
    mutationFn: () => questAdminClient().revokeBadge(target),
    onSuccess: (response) => {
      afterAction();
      toast.success(t(response.revoked ? "admin.revoked" : "admin.notOwned"));
    },
    onError,
  });

  const search = (event: FormEvent) => {
    event.preventDefault();
    const value = handle.trim().replace(/^@/, "");
    if (value) find.mutate(value);
  };

  // Special badges first: they are what is given by hand. Unearned secrets have no real key here.
  const families = [...(catalog.data?.families ?? [])].sort(
    (a, b) => Number(b.key === "special") - Number(a.key === "special"),
  );
  const options = families.flatMap((family) =>
    family.badges
      .filter((badge) => !isHiddenSecret(badge))
      .map((badge) => ({
        value: badge.key,
        label: `${badge.emoji} ${badgeName(badge)} (${text(family.name)})`,
      })),
  );
  // The server counts characters, not UTF-16 units.
  const tooLong = Array.from(reason).length > REASON_LIMIT;
  const ready = Boolean(player && badgeKey && reason.trim()) && !tooLong;

  return (
    <div className="flex flex-col gap-7">
      <section aria-label={t("admin.give")}>
        <SectionTitle>{t("admin.give")}</SectionTitle>
        <p className="mt-1 text-sm text-ink-soft">{t("admin.giveHint")}</p>

        <form onSubmit={search} className="mt-4 flex items-end gap-2">
          <TextField
            className="flex-1"
            label={t("admin.player")}
            placeholder="@handle"
            value={handle}
            onChange={(event) => setHandle(event.target.value)}
            autoCapitalize="none"
            autoCorrect="off"
          />
          <Button type="submit" variant="secondary" loading={find.isPending}>
            {t("admin.find")}
          </Button>
        </form>
        {player && (
          <p className="mt-3 flex items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2">
            <Avatar user={player} size="sm" />
            <span className="min-w-0 flex-1 truncate font-bold">
              <EmojiText text={player.displayName} />{" "}
              <span className="text-ink-soft">@{player.handle}</span>
            </span>
          </p>
        )}

        <SelectField
          className="mt-4"
          label={t("admin.badge")}
          value={badgeKey}
          onChange={setBadgeKey}
          disabled={catalog.isPending}
          options={[{ value: "", label: t("admin.pickBadge") }, ...options]}
        />
        <TextArea
          className="mt-4"
          label={t("admin.reason")}
          hint={t("admin.reasonHint")}
          error={tooLong ? t("errors.REASON_TOO_LONG") : undefined}
          value={reason}
          onChange={(event) => setReason(event.target.value)}
        />
        <div className="mt-4 flex flex-wrap gap-2">
          <Button disabled={!ready} loading={award.isPending} onClick={() => award.mutate()}>
            {t("admin.award")}
          </Button>
          <Button
            variant="danger"
            disabled={!ready}
            loading={revoke.isPending}
            onClick={() => revoke.mutate()}
          >
            {t("admin.revoke")}
          </Button>
        </div>
      </section>

      <ActionLog player={player} />
    </div>
  );
}

function ActionLog({ player }: { player: common.UserSummary | null }) {
  const { t, i18n } = useTranslation("quest");
  const userId = player?.id ?? "";
  const { data, isPending, isError } = useQuery({
    queryKey: questKeys.admin.actions(userId),
    queryFn: async () =>
      (await questAdminClient().listBadgeActions({ userId, limit: 100 })).actions,
  });
  return (
    <section aria-label={t("admin.log")}>
      <SectionTitle aside={player ? `@${player.handle}` : t("admin.logAll")}>
        {t("admin.log")}
      </SectionTitle>
      {isPending ? (
        <Skeleton className="mt-3 h-24" />
      ) : isError ? (
        <EmptyState emoji="🛰️" title={t("admin.logError")} />
      ) : data.length === 0 ? (
        <p className="mt-2 text-sm font-bold text-ink-soft">{t("admin.logEmpty")}</p>
      ) : (
        <ul className="sticker mt-3 divide-y-2 divide-line rounded-card bg-surface px-4">
          {data.map((action) => {
            const awarded = action.kind === quest.BadgeActionKind.AWARDED;
            return (
              <li key={action.id} className="py-3 text-sm">
                <p className="flex flex-wrap items-center gap-x-2 font-bold">
                  <span
                    className={cn(
                      "rounded-full px-2 py-0.5 text-[11px] font-black",
                      awarded ? "bg-lime text-[#17122b]" : "bg-danger text-white",
                    )}
                  >
                    {t(awarded ? "admin.kindAwarded" : "admin.kindRevoked")}
                  </span>
                  <code className="font-mono">{action.badgeKey}</code>
                  {!action.changed && (
                    <span className="text-ink-faint">{t("admin.unchanged")}</span>
                  )}
                </p>
                <p className="mt-1 break-words whitespace-pre-wrap">{action.reason}</p>
                <p className="mt-1 text-xs break-all text-ink-faint">
                  {toDate(action.createdAt)?.toLocaleString(i18n.language)} ·{" "}
                  {t("admin.logPlayer", { id: action.userId })} ·{" "}
                  {action.staffId
                    ? t("admin.logStaff", { id: action.staffId })
                    : t("admin.logStaffGone")}
                </p>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
