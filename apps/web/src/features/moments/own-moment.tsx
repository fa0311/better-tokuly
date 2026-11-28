import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { moments } from "@tokuly/api";
import { latenessText, removeMoment } from "@tokuly/core/moments";
import { Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { momentClient } from "./api";
import { MomentPair } from "./pair";
import { ReactionStrip } from "./reactions";

/** The caller's own moment of the day, with its reactions and a way to delete it. */
export function OwnMoment({ moment }: { moment: moments.Moment }) {
  const { t, i18n } = useTranslation("moments");
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const remove = useMutation({
    mutationFn: () => momentClient().deleteMoment({ momentId: moment.id }),
    onSuccess: () => {
      removeMoment(queryClient, moment.id);
      setConfirming(false);
      toast.success(t("card.deleted"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  return (
    <section
      className="sticker rounded-card bg-surface p-4"
      data-testid="moment-day"
      data-phase="posted"
    >
      <div className="mb-3 flex items-center gap-2">
        <h2 className="flex-1 text-lg font-black">{t("today.postedTitle")}</h2>
        <span className="rounded-full bg-lime px-2 py-0.5 text-[11px] font-black text-[#17122b]">
          {latenessText(i18n, moment.lateSeconds)}
        </span>
      </div>
      <div className="flex gap-4">
        <MomentPair moment={moment} width={160} className="w-36 shrink-0 sm:w-44" />
        <div className="flex min-w-0 flex-1 flex-col gap-2">
          {moment.caption && (
            <p className="font-bold">
              <EmojiText text={moment.caption} />
            </p>
          )}
          <ReactionStrip moment={moment} />
          <Link
            to="/moments/$momentId"
            params={{ momentId: moment.id }}
            className="text-xs font-black text-brand"
          >
            {t("card.seeAll")} →
          </Link>
          <Button
            variant="ghost"
            size="sm"
            className="mt-auto self-start text-danger"
            icon={<Trash2 className="size-4" />}
            onClick={() => setConfirming(true)}
          >
            {t("card.delete")}
          </Button>
        </div>
      </div>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("card.deleteTitle")}
        description={t("card.deleteBody")}
        confirmLabel={t("card.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </section>
  );
}
