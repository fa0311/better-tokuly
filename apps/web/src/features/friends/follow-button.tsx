import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type common, social, toDate } from "@tokuly/api";
import { BellRing, Check, ChevronDown, Clock, UserMinus, UserPlus, VolumeX } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { followClient, followKeys, refreshFollows, useFollowSummary } from "./follow-data";

const NS = { namespace: "follows" };

/** Mute lengths offered, in seconds (0 = until unmuted). */
const MUTE_SECONDS = [
  { key: "hour", seconds: 3600 },
  { key: "day", seconds: 86_400 },
  { key: "week", seconds: 604_800 },
  { key: "forever", seconds: 0 },
] as const;

/**
 * Follow / フォロー中 / リクエスト済み for one person. Following someone opens
 * a sheet with their notifications, mute and unfollow; a pending request can
 * be withdrawn. Renders nothing for the viewer and for someone they blocked.
 */
export function FollowButton({
  user,
  relation,
  locked,
  size = "sm",
}: {
  user: common.UserSummary;
  relation: social.FollowRelation | undefined;
  /** The account approves its followers (known on profiles; lists learn it on follow). */
  locked?: boolean;
  size?: "sm" | "md";
}) {
  const { t } = useTranslation("follows");
  const queryClient = useQueryClient();
  const shown = relation?.following ?? social.FollowState.NONE;
  // What the last tap made of it, until the lists that carry `relation` have re-read.
  const [override, setOverride] = useState<{ from: social.FollowState; to: social.FollowState }>();
  const state = override?.from === shown ? override.to : shown;
  const [sheet, setSheet] = useState(false);
  const [withdrawing, setWithdrawing] = useState(false);

  const follow = useMutation({
    mutationFn: () => followClient().follow({ target: { case: "userId", value: user.id } }),
    onSuccess: (response) => {
      setOverride({ from: shown, to: response.state });
      toast.success(
        response.state === social.FollowState.REQUESTED
          ? t("toast.requested", { name: user.displayName })
          : t("toast.followed", { name: user.displayName }),
      );
      return refreshFollows(queryClient);
    },
    onError: (error) => toast.error(errorMessage(error, NS)),
  });
  const unfollow = useMutation({
    mutationFn: () => followClient().unfollow({ userId: user.id }),
    onSuccess: () => {
      setOverride({ from: shown, to: social.FollowState.NONE });
      setSheet(false);
      setWithdrawing(false);
      toast.success(
        state === social.FollowState.REQUESTED ? t("toast.withdrawn") : t("toast.unfollowed"),
      );
      return refreshFollows(queryClient);
    },
    onError: (error) => toast.error(errorMessage(error, NS)),
  });

  if (!relation || relation.self || relation.blocked) return null;

  if (state === social.FollowState.FOLLOWING)
    return (
      <>
        <Button
          size={size}
          variant="secondary"
          icon={<Check className="size-4" />}
          aria-haspopup="dialog"
          onClick={() => setSheet(true)}
        >
          {t("button.following")}
          <ChevronDown className="size-4" aria-hidden />
        </Button>
        <FollowingSheet
          open={sheet}
          onOpenChange={setSheet}
          user={user}
          locked={locked}
          unfollowing={unfollow.isPending}
          onUnfollow={() => unfollow.mutate()}
        />
      </>
    );

  if (state === social.FollowState.REQUESTED)
    return (
      <>
        <Button size={size} variant="secondary" onClick={() => setWithdrawing(true)}>
          {t("button.requested")}
        </Button>
        <ConfirmDialog
          open={withdrawing}
          onOpenChange={setWithdrawing}
          title={t("withdraw.title")}
          description={t("withdraw.body", { name: user.displayName })}
          confirmLabel={t("withdraw.confirm")}
          loading={unfollow.isPending}
          onConfirm={() => unfollow.mutate()}
        />
      </>
    );

  return (
    <Button
      size={size}
      icon={<UserPlus className="size-4" />}
      loading={follow.isPending}
      onClick={() => follow.mutate()}
    >
      {relation.followsYou ? t("button.followBack") : t("button.follow")}
    </Button>
  );
}

/** What following someone can be tuned with: the bell, mute, and unfollow. */
function FollowingSheet({
  open,
  onOpenChange,
  user,
  locked,
  unfollowing,
  onUnfollow,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  user: common.UserSummary;
  locked?: boolean;
  unfollowing: boolean;
  onUnfollow(): void;
}) {
  const { t, i18n } = useTranslation("follows");
  const queryClient = useQueryClient();
  // The prefs are the viewer's own: read them when the sheet opens.
  const summary = useFollowSummary(open ? user.id : undefined);
  const prefs = summary.data?.prefs;
  const isLocked = locked ?? summary.data?.locked ?? false;

  const update = useMutation({
    mutationFn: (input: {
      muted?: boolean;
      muteSeconds?: bigint;
      notifyLevel?: social.FollowNotifyLevel;
    }) => followClient().updateFollowPrefs({ userId: user.id, ...input }),
    onSuccess: (response, input) => {
      queryClient.setQueryData<social.FollowSummary | undefined>(
        followKeys.summary(user.id),
        (current) => (current ? { ...current, prefs: response.prefs } : current),
      );
      if (input.muted !== undefined)
        toast.success(input.muted ? t("toast.muted") : t("toast.unmuted"));
      return queryClient.invalidateQueries({ queryKey: followKeys.lists });
    },
    onError: (error) => toast.error(errorMessage(error, NS)),
  });

  const mutedUntil = prefs?.mutedUntil ? toDate(prefs.mutedUntil) : undefined;

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("sheet.title", { name: user.displayName })}
      description={t("sheet.lead")}
    >
      <div className="flex flex-col gap-5">
        <SwitchRow
          label={
            <span className="inline-flex items-center gap-2">
              <BellRing className="size-4" aria-hidden />
              {t("sheet.notify")}
            </span>
          }
          hint={t("sheet.notifyHint")}
          checked={prefs?.notifyLevel === social.FollowNotifyLevel.ALL}
          disabled={!prefs || update.isPending}
          onChange={(on) =>
            update.mutate({
              notifyLevel: on ? social.FollowNotifyLevel.ALL : social.FollowNotifyLevel.NONE,
            })
          }
        />

        <section aria-labelledby="follow-mute-title" className="px-3">
          <h3 id="follow-mute-title" className="flex items-center gap-2 font-bold">
            <VolumeX className="size-4" aria-hidden />
            {t("sheet.mute")}
          </h3>
          <p className="text-sm text-ink-soft">{t("sheet.muteHint")}</p>
          {prefs?.muted ? (
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <span className="inline-flex items-center gap-1.5 rounded-full bg-sun px-3 py-1 text-sm font-black text-[#17122b]">
                <Clock className="size-4" aria-hidden />
                {mutedUntil
                  ? t("sheet.mutedUntil", {
                      when: mutedUntil.toLocaleString(i18n.language, {
                        month: "short",
                        day: "numeric",
                        hour: "2-digit",
                        minute: "2-digit",
                      }),
                    })
                  : t("sheet.mutedForever")}
              </span>
              <Button
                size="sm"
                variant="secondary"
                loading={update.isPending}
                onClick={() => update.mutate({ muted: false })}
              >
                {t("sheet.unmute")}
              </Button>
            </div>
          ) : (
            <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-4">
              {MUTE_SECONDS.map((option) => (
                <Button
                  key={option.key}
                  size="sm"
                  variant="secondary"
                  disabled={!prefs || update.isPending}
                  onClick={() =>
                    update.mutate({ muted: true, muteSeconds: BigInt(option.seconds) })
                  }
                >
                  {t(`sheet.muteFor.${option.key}`)}
                </Button>
              ))}
            </div>
          )}
        </section>

        <div className="border-t-2 border-line pt-4">
          {isLocked && <p className="mb-2 px-3 text-sm text-ink-soft">{t("sheet.lockedNote")}</p>}
          <Button
            block
            variant="danger"
            icon={<UserMinus className="size-4" />}
            loading={unfollowing}
            onClick={onUnfollow}
          >
            {t("sheet.unfollow")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
