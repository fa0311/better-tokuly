import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { spaces } from "@tokuly/api";
import { forgetHangout, insideOf, nameOf, rememberHangout, spacesKeys } from "@tokuly/core/spaces";
import { BellOff, Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { hangoutClient, useHangouts } from "../api";
import { Faces } from "../parts";
import { HangoutForm } from "./hangout-form";

/** The person's hangouts, open ones first, and the invitations waiting for an answer. */
export function HangoutsTab() {
  const { t } = useTranslation("spaces");
  const board = useHangouts();
  const [creating, setCreating] = useState(false);

  if (board.isPending)
    return (
      <div className="grid gap-4 md:grid-cols-2">
        <Skeleton className="h-28" />
        <Skeleton className="h-28" />
      </div>
    );
  const hangouts = board.data?.hangouts ?? [];
  const invitations = board.data?.invitations ?? [];
  return (
    <div className="flex flex-col gap-6">
      {invitations.length > 0 && (
        <section className="flex flex-col gap-3" data-testid="hangout-invitations">
          <h2 className="text-lg font-black">{t("hangouts.invitations")}</h2>
          <div className="grid gap-4 md:grid-cols-2">
            {invitations.map((hangout) => (
              <Invitation key={hangout.id} hangout={hangout} />
            ))}
          </div>
        </section>
      )}
      {hangouts.length === 0 ? (
        <EmptyState
          emoji="🛋️"
          title={t("hangouts.empty")}
          body={t("hangouts.emptyBody")}
          action={
            <Button icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
              {t("hangouts.create")}
            </Button>
          }
        />
      ) : (
        <section className="flex flex-col gap-3">
          <Button
            variant="secondary"
            className="self-start"
            icon={<Plus className="size-4" />}
            onClick={() => setCreating(true)}
          >
            {t("hangouts.create")}
          </Button>
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {hangouts.map((hangout) => (
              <HangoutCard key={hangout.id} hangout={hangout} />
            ))}
          </div>
        </section>
      )}
      <HangoutForm open={creating} onOpenChange={setCreating} />
    </div>
  );
}

function HangoutCard({ hangout }: { hangout: spaces.Hangout }) {
  const { t } = useTranslation("spaces");
  const inside = insideOf(hangout).flatMap((member) => (member.user ? [member.user] : []));
  const open = hangout.insideCount > 0;
  return (
    <Link
      to="/spaces/hangouts/$hangoutId"
      params={{ hangoutId: hangout.id }}
      className={cn(
        "pressable sticker flex items-center gap-3 rounded-card p-4",
        open ? "bg-lime text-[#17122b]" : "bg-surface",
      )}
      data-testid="hangout-card"
      data-open={open || undefined}
    >
      <span className="grid size-14 shrink-0 place-items-center rounded-2xl bg-surface text-3xl leading-none">
        {hangout.emoji || "🛋️"}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="truncate text-lg font-black">
            <EmojiText text={hangout.name} />
          </span>
          {hangout.viewer?.muted && (
            <BellOff className="size-4 shrink-0 opacity-60" aria-label={t("hangout.mutedLabel")} />
          )}
        </span>
        {open ? (
          <span className="mt-1 flex items-center gap-2 text-sm font-bold">
            <Faces users={inside.slice(0, 4)} size="xs" />
            {t("hangouts.inside", { count: hangout.insideCount })}
          </span>
        ) : (
          <span className={cn("mt-1 block text-sm font-bold", !open && "text-ink-soft")}>
            {t("hangouts.nobody", { count: hangout.memberCount })}
          </span>
        )}
      </span>
    </Link>
  );
}

function Invitation({ hangout }: { hangout: spaces.Hangout }) {
  const { t } = useTranslation("spaces");
  const respond = useRespondToInvite(hangout.id);
  return (
    <div className="sticker flex flex-col gap-3 rounded-card bg-brand-soft p-4">
      <Link
        to="/spaces/hangouts/$hangoutId"
        params={{ hangoutId: hangout.id }}
        className="flex items-center gap-3"
      >
        <span className="grid size-12 shrink-0 place-items-center rounded-2xl bg-surface text-2xl leading-none">
          {hangout.emoji || "🛋️"}
        </span>
        <span className="min-w-0">
          <span className="block truncate font-black">
            <EmojiText text={hangout.name} />
          </span>
          <span className="block truncate text-sm font-bold text-ink-soft">
            {t("hangouts.invitedBy", { name: nameOf(hangout.owner), count: hangout.memberCount })}
          </span>
        </span>
      </Link>
      <div className="flex gap-2">
        <Button
          size="sm"
          variant="lime"
          loading={respond.isPending && respond.variables === true}
          disabled={respond.isPending}
          onClick={() => respond.mutate(true)}
        >
          {t("hangouts.accept")}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={respond.isPending}
          onClick={() => respond.mutate(false)}
        >
          {t("hangouts.decline")}
        </Button>
      </div>
    </div>
  );
}

/** Accepts or declines an invitation to a hangout. */
export function useRespondToInvite(hangoutId: string) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (accept: boolean) => hangoutClient().respondToHangoutInvite({ hangoutId, accept }),
    onSuccess: (response, accept) => {
      if (accept) {
        rememberHangout(queryClient, response.hangout);
        toast.success(t("toast.hangoutJoined"));
      } else forgetHangout(queryClient, hangoutId);
      void queryClient.invalidateQueries({ queryKey: spacesKeys.hangouts });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "spaces" })),
  });
}
