import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { notify } from "@tokuly/api";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import { useSessionStore } from "#/stores/session";
import { notifyKeys, pushClient } from "./api";
import { currentSubscription, disablePush, enablePush, PushError, type PushOwner, pushSupported } from "./push";

/**
 * Push on this browser, for the signed-in account:
 * - `unsupported`: the browser has no Web Push (or it needs installing first, on iOS)
 * - `unavailable`: the server offers no Web Push
 * - `denied`: notifications are blocked for this site in the browser
 * - `off`: could be turned on with one tap
 * - `on`: this browser is registered and will receive pushes
 */
export type PushStatus = "loading" | "unsupported" | "unavailable" | "denied" | "off" | "on";

const subscriptionKey = [...notifyKeys.devices, "subscription"] as const;

function usePermission(): { permission: NotificationPermission; recheck(): void } {
  const [permission, setPermission] = useState<NotificationPermission>(() => (pushSupported() ? Notification.permission : "denied"));
  const recheck = () => {
    if (pushSupported()) setPermission(Notification.permission);
  };
  useEffect(() => {
    if (!pushSupported()) return;
    // The answer can change behind our back, in the browser's site settings.
    const update = () => setPermission(Notification.permission);
    let status: PermissionStatus | undefined;
    let gone = false;
    void navigator.permissions
      ?.query({ name: "notifications" })
      .then((result) => {
        if (gone) return;
        status = result;
        status.addEventListener("change", update);
      })
      .catch(() => {});
    window.addEventListener("focus", update);
    return () => {
      gone = true;
      status?.removeEventListener("change", update);
      window.removeEventListener("focus", update);
    };
  }, []);
  return { permission, recheck };
}

/** The session push registrations are made for. */
function useOwner(): PushOwner | null {
  const userId = useSessionStore((s) => (s.state.status === "signedIn" ? (s.state.me.user?.id ?? "") : ""));
  const sessionId = useSessionStore((s) => (s.state.status === "signedIn" ? s.state.sessionId : ""));
  return userId ? { userId, sessionId } : null;
}

/** State and actions of Web Push for this browser and of the account's registered devices. */
export function usePush() {
  const { t } = useTranslation("notifications");
  const queryClient = useQueryClient();
  const owner = useOwner();
  const supported = pushSupported();
  const { permission, recheck } = usePermission();

  const config = useQuery({
    queryKey: notifyKeys.pushConfig,
    queryFn: () => pushClient().getPushConfig({}),
    staleTime: Number.POSITIVE_INFINITY,
    enabled: supported,
  });
  const devices = useQuery({ queryKey: notifyKeys.devices, queryFn: async () => (await pushClient().listDevices({})).devices });
  const subscription = useQuery({
    queryKey: subscriptionKey,
    queryFn: async () => (await currentSubscription())?.endpoint ?? null,
    enabled: supported,
    staleTime: 0,
  });

  const thisDevice = devices.data?.find((device) => device.current && device.channel === notify.PushChannelKind.WEB_PUSH);
  const offered = Boolean(config.data?.vapidPublicKey) && config.data?.channels.includes(notify.PushChannelKind.WEB_PUSH);

  let status: PushStatus;
  if (!supported) status = "unsupported";
  else if (config.isPending || devices.isPending) status = "loading";
  else if (!offered) status = "unavailable";
  else if (permission === "denied") status = "denied";
  else if (permission !== "granted" || !thisDevice) status = "off";
  // Registered for this session: it is on if the browser still holds its subscription,
  // which only the service worker can tell (and a cold one takes a moment to start).
  else if (subscription.isPending) status = "loading";
  else status = subscription.data ? "on" : "off";

  const refresh = () => {
    recheck();
    return queryClient.invalidateQueries({ queryKey: notifyKeys.devices });
  };

  const enable = useMutation({
    mutationFn: async () => {
      if (!owner) throw new PushError("unsupported");
      return enablePush(owner);
    },
    onSuccess: () => toast.success(t("push.toast.enabled")),
    onError: (error) => {
      // Closing the browser's prompt is an answer, not a problem.
      if (error instanceof PushError && error.failure === "dismissed") return;
      toast.error(error instanceof PushError ? t(`push.failure.${error.failure}`) : errorMessage(error));
    },
    onSettled: refresh,
  });

  const disable = useMutation({
    mutationFn: disablePush,
    onSuccess: () => toast.success(t("push.toast.disabled")),
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: refresh,
  });

  /** Removes another device's registration (this browser's goes through `disable`). */
  const removeDevice = useMutation({
    mutationFn: (deviceId: string) => pushClient().unregisterDevice({ deviceId }),
    onSuccess: () => toast.success(t("push.toast.deviceRemoved")),
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: refresh,
  });

  const sendTest = useMutation({
    mutationFn: () => pushClient().sendTestPush({}),
    onSuccess: (response) => toast.success(t("push.toast.testSent", { count: response.deviceCount })),
    onError: (error) => toast.error(errorMessage(error)),
    // A device whose subscription turned out to be dead is dropped by the server right after.
    onSettled: () => setTimeout(refresh, 3000),
  });

  return { status, devices: devices.data ?? [], devicesPending: devices.isPending, thisDevice, enable, disable, removeDevice, sendTest };
}
