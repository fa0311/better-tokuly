import { useMutation, useQueryClient } from "@tanstack/react-query";
import { moments } from "@tokuly/api";
import { momentKeys } from "@tokuly/core/moments";
import { toast } from "sonner";
import { Dialog } from "#/components/ui/dialog";
import { Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { momentClient, useMomentSettings } from "./api";

type Audience = "friends" | "close";

/** The daily prompt on or off, and what a new moment starts with: its place, who sees it. */
export function MomentSettingsDialog({ open, onClose }: { open: boolean; onClose(): void }) {
  const { t } = useTranslation("moments");
  const queryClient = useQueryClient();
  const settings = useMomentSettings();
  const update = useMutation({
    mutationFn: (change: {
      shareLocation?: boolean;
      prompts?: boolean;
      audience?: moments.MomentAudience;
    }) => momentClient().updateMomentSettings(change),
    onSuccess: ({ settings: saved }) => {
      queryClient.setQueryData(momentKeys.settings, saved);
      void queryClient.invalidateQueries({ queryKey: momentKeys.today });
      toast.success(t("settings.saved"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  const value = settings.data;
  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()} title={t("settings.title")}>
      {!value ? (
        <Skeleton className="h-40" />
      ) : (
        <div className="flex flex-col gap-4">
          <SwitchRow
            label={t("settings.prompts")}
            hint={t("settings.promptsHint")}
            checked={value.prompts}
            disabled={update.isPending}
            onChange={(prompts) => update.mutate({ prompts })}
          />
          <SwitchRow
            label={t("settings.location")}
            hint={t("settings.locationHint")}
            checked={value.shareLocation}
            disabled={update.isPending}
            onChange={(shareLocation) => update.mutate({ shareLocation })}
          />
          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-bold">{t("settings.audience")}</span>
            <Tabs<Audience>
              value={value.audience === moments.MomentAudience.CLOSE_FRIENDS ? "close" : "friends"}
              onChange={(next) =>
                update.mutate({
                  audience:
                    next === "close"
                      ? moments.MomentAudience.CLOSE_FRIENDS
                      : moments.MomentAudience.FRIENDS,
                })
              }
              items={[
                { value: "friends", label: t("camera.friends") },
                { value: "close", label: t("camera.closeFriends") },
              ]}
            />
          </div>
        </div>
      )}
    </Dialog>
  );
}
