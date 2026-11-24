import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { common, type identity, toMillis } from "@tokuly/api";
import { Globe, Laptop, Smartphone } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Skeleton } from "#/components/ui/page";
import { accountClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import { keys } from "../data";
import { Group, StateChip } from "../shell/settings-page";

function PlatformIcon({ platform }: { platform: common.Platform }) {
  if (platform === common.Platform.IOS || platform === common.Platform.ANDROID)
    return <Smartphone className="size-5" />;
  if (platform === common.Platform.DESKTOP) return <Laptop className="size-5" />;
  return <Globe className="size-5" />;
}

/** What to sign out: one device, or every device but this one. */
type Target =
  | { kind: "one"; session: identity.Session }
  | { kind: "others"; sessions: identity.Session[] };

/** Devices signed in to the account, with the current one marked; any other can be signed out. */
export function SessionsCard() {
  const { t } = useTranslation("settings");
  const queryClient = useQueryClient();
  const { data, isPending } = useQuery({
    queryKey: keys.sessions,
    queryFn: () => accountClient().listSessions({}),
  });
  const [confirming, setConfirming] = useState<{ target: Target | null; open: boolean }>({
    target: null,
    open: false,
  });
  const close = () => setConfirming((current) => ({ ...current, open: false }));

  const revoke = useMutation({
    mutationFn: (target: Target) => {
      const sessions = target.kind === "one" ? [target.session] : target.sessions;
      return Promise.all(
        sessions.map((session) => accountClient().revokeSession({ sessionId: session.id })),
      );
    },
    onSuccess: (_, target) =>
      toast.success(
        target.kind === "one"
          ? t("security.sessions.revoked")
          : t("security.sessions.revokedOthers"),
      ),
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: () => {
      close();
      return Promise.all([
        queryClient.invalidateQueries({ queryKey: keys.sessions }),
        queryClient.invalidateQueries({ queryKey: keys.securityEvents }),
      ]);
    },
  });

  // This device first, then the most recently used.
  const sessions = [...(data?.sessions ?? [])].sort(
    (a, b) =>
      Number(b.current) - Number(a.current) || toMillis(b.lastActiveAt) - toMillis(a.lastActiveAt),
  );
  const others = sessions.filter((session) => !session.current);
  const target = confirming.target;

  return (
    <Group title={t("security.sessions.title")} lead={t("security.sessions.lead")} flush>
      {isPending ? (
        <Skeleton className="m-1 h-14" />
      ) : (
        <ul>
          {sessions.map((session) => {
            const name = session.client?.deviceName || t("security.unknownDevice");
            const place = session.location || session.ip;
            return (
              <li
                key={session.id}
                data-testid="session-row"
                className="flex items-center gap-3 rounded-2xl px-3 py-2.5"
              >
                <span className="grid size-10 shrink-0 place-items-center rounded-xl border-2 border-ink bg-sky text-[#17122b]">
                  <PlatformIcon
                    platform={session.client?.platform ?? common.Platform.UNSPECIFIED}
                  />
                </span>
                <div className="min-w-0 flex-1">
                  <p className="truncate font-bold">{name}</p>
                  {(session.addedBySwitch || session.parked) && (
                    <p className="mt-0.5 flex flex-wrap gap-1">
                      {session.addedBySwitch && (
                        <StateChip tone="off">{t("security.sessions.addedBySwitch")}</StateChip>
                      )}
                      {session.parked && (
                        <StateChip tone="warn">{t("security.sessions.parked")}</StateChip>
                      )}
                    </p>
                  )}
                  {/* Where it signed in from, then when it was last used; a line break only falls between the two. */}
                  <p className="text-sm leading-snug text-ink-soft">
                    {place && <span className="whitespace-nowrap">{place} ・ </span>}
                    <span className="whitespace-nowrap">
                      {session.current
                        ? t("security.sessions.activeNow")
                        : t("security.sessions.lastActive", {
                            age: compactAge(toMillis(session.lastActiveAt)),
                          })}
                    </span>
                  </p>
                </div>
                {session.current ? (
                  <StateChip tone="good">{t("security.sessions.current")}</StateChip>
                ) : (
                  <Button
                    size="sm"
                    variant="secondary"
                    className="shrink-0"
                    onClick={() => setConfirming({ target: { kind: "one", session }, open: true })}
                  >
                    {t("security.sessions.revoke")}
                  </Button>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {others.length > 1 && (
        <div className="px-2 pt-2 pb-1">
          <Button
            size="sm"
            variant="ghost"
            className="text-danger"
            onClick={() =>
              setConfirming({ target: { kind: "others", sessions: others }, open: true })
            }
          >
            {t("security.sessions.revokeOthers")}
          </Button>
        </div>
      )}
      <ConfirmDialog
        open={confirming.open}
        onOpenChange={(open) => !open && close()}
        title={
          target?.kind === "others"
            ? t("security.sessions.revokeOthersTitle")
            : t("security.sessions.revokeTitle", {
                name:
                  (target?.kind === "one" && target.session.client?.deviceName) ||
                  t("security.unknownDevice"),
              })
        }
        description={t("security.sessions.revokeBody")}
        confirmLabel={t("security.sessions.revokeConfirm")}
        danger
        loading={revoke.isPending}
        onConfirm={() => target && revoke.mutate(target)}
      />
    </Group>
  );
}
