import { common, notify, toMillis } from "@tokuly/api";
import { BellOff, BellRing, Monitor, Send, Smartphone, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { needsHomeScreenInstall } from "../push";
import { type PushStatus, usePush } from "../use-push";
import { formatWhen, useNow } from "../when";

const CHANNEL_NAMES: Record<number, string> = {
  [notify.PushChannelKind.WEB_PUSH]: "Web Push",
  [notify.PushChannelKind.FCM]: "Android",
  [notify.PushChannelKind.APNS]: "iOS",
};

/** Push on this browser (on / off / why not), and every device of the account that receives pushes. */
export function PushCard() {
  const { t } = useTranslation("notifications");
  const push = usePush();
  const now = useNow();
  const [removing, setRemoving] = useState<notify.PushDevice | null>(null);
  const on = push.status === "on";

  return (
    <Card>
      <div className="flex items-start gap-3">
        <span
          className={cn(
            "sticker-sm grid size-12 shrink-0 place-items-center rounded-2xl text-[#17122b]",
            on ? "bg-lime" : "bg-surface-3 text-ink",
          )}
        >
          {on ? <BellRing className="size-6" /> : <BellOff className="size-6" />}
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="flex flex-wrap items-center gap-2 text-lg font-black leading-snug">
            {t("push.title")}
            {push.status !== "loading" && (
              <Chip className={cn(on && "bg-lime text-[#17122b]")}>
                {t(`push.status.${push.status}`)}
              </Chip>
            )}
          </h2>
          {push.status === "loading" ? (
            <Skeleton className="mt-2 h-10" />
          ) : (
            <p className="mt-1 text-sm leading-relaxed text-ink-soft">{t(lead(push.status))}</p>
          )}
        </div>
      </div>

      {(push.status === "off" || on) && (
        <div className="mt-4 flex flex-wrap gap-2">
          {on ? (
            <Button
              variant="secondary"
              size="sm"
              loading={push.disable.isPending}
              onClick={() => push.disable.mutate()}
            >
              {t("push.disable")}
            </Button>
          ) : (
            <Button
              icon={<BellRing className="size-4" />}
              loading={push.enable.isPending}
              onClick={() => push.enable.mutate()}
            >
              {t("push.enable")}
            </Button>
          )}
        </div>
      )}

      <div className="mt-5 border-t-2 border-line pt-4">
        <div className="flex items-center gap-2">
          <h3 className="min-w-0 flex-1 text-sm font-black text-ink-soft">{t("devices.title")}</h3>
          <Button
            variant="secondary"
            size="sm"
            icon={<Send className="size-4" />}
            disabled={push.devices.length === 0}
            loading={push.sendTest.isPending}
            onClick={() => push.sendTest.mutate()}
          >
            {t("devices.test")}
          </Button>
        </div>
        {push.devicesPending ? (
          <Skeleton className="mt-3 h-14" />
        ) : push.devices.length === 0 ? (
          <p className="mt-2 text-sm text-ink-faint">{t("devices.empty")}</p>
        ) : (
          <ul className="mt-2 flex flex-col">
            {push.devices.map((device) => {
              const Icon =
                device.platform === common.Platform.IOS ||
                device.platform === common.Platform.ANDROID
                  ? Smartphone
                  : Monitor;
              const delivered = toMillis(device.lastDeliveredAt);
              return (
                <li key={device.id} className="flex items-center gap-3 py-2">
                  <span className="grid size-10 shrink-0 place-items-center rounded-xl bg-surface-2 text-ink-soft">
                    <Icon className="size-5" />
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="flex items-center gap-2 font-bold">
                      <span className="min-w-0 truncate">
                        {device.deviceName || t("devices.unnamed")}
                      </span>
                      {device.current && (
                        <Chip className="shrink-0 bg-brand-soft text-brand-strong">
                          {t("devices.current")}
                        </Chip>
                      )}
                    </p>
                    <p className="truncate text-xs text-ink-faint">
                      {[
                        CHANNEL_NAMES[device.channel],
                        t("devices.added", {
                          when: formatWhen(toMillis(device.createdAt), now),
                        }),
                        delivered
                          ? t("devices.delivered", {
                              when: formatWhen(delivered, now),
                            })
                          : t("devices.neverDelivered"),
                      ]
                        .filter(Boolean)
                        .join(" ・ ")}
                    </p>
                  </div>
                  {/* This browser is switched off above, which also drops its subscription. */}
                  {!device.current && (
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="shrink-0 text-ink-soft"
                      aria-label={t("devices.remove", { name: device.deviceName })}
                      onClick={() => setRemoving(device)}
                    >
                      <X className="size-4" />
                    </Button>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>

      <ConfirmDialog
        open={removing !== null}
        onOpenChange={(open) => !open && setRemoving(null)}
        title={t("devices.removeTitle", { name: removing?.deviceName || t("devices.unnamed") })}
        description={t("devices.removeBody")}
        confirmLabel={t("devices.removeConfirm")}
        danger
        loading={push.removeDevice.isPending}
        onConfirm={() =>
          removing && push.removeDevice.mutate(removing.id, { onSettled: () => setRemoving(null) })
        }
      />
    </Card>
  );
}

/** The explanation under the title, per state. */
function lead(status: Exclude<PushStatus, "loading">): string {
  if (status === "unsupported")
    return needsHomeScreenInstall() ? "push.lead.install" : "push.lead.unsupported";
  return `push.lead.${status}`;
}
