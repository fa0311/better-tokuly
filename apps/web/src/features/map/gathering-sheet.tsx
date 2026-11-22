import { toMillis } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { compactDuration } from "#/lib/format";
import { useMe } from "#/stores/session";
import { useGatherings, useNow } from "./data";
import { ReactionBar } from "./reaction-bar";
import { MapSheet, SheetSection } from "./sheet";
import { useMapUi } from "./ui-store";

/** Who is in a gathering, for how long, and a way to throw something at all of them at once. */
export function GatheringSheet({ id }: { id: string }) {
  const { t } = useTranslation("map");
  const { data } = useGatherings();
  const myId = useMe()?.user?.id;
  const now = useNow(30_000);
  const { stack, back, close, push } = useMapUi();
  const gathering = data?.find((entry) => entry.id === id);

  if (!gathering) {
    return (
      <MapSheet
        testId="gathering-sheet"
        title={t("gathering.over")}
        onBack={stack.length > 1 ? back : undefined}
        onClose={close}
      >
        <p className="py-6 text-center text-sm text-ink-soft">{t("gathering.overBody")}</p>
      </MapSheet>
    );
  }
  const others = gathering.members.filter((member) => member.user && member.user.id !== myId);
  return (
    <MapSheet
      testId="gathering-sheet"
      title={t("gathering.count", { count: gathering.members.length })}
      subtitle={[
        gathering.placeName,
        t("gathering.since", { duration: compactDuration(now - toMillis(gathering.startedAt)) }),
      ]
        .filter(Boolean)
        .join(" · ")}
      leading={<span className="text-2xl">{gathering.atPrivatePlace ? "🏠" : "🔥"}</span>}
      onBack={stack.length > 1 ? back : undefined}
      onClose={close}
    >
      <SheetSection title={t("gathering.members")}>
        <ul className="space-y-1">
          {gathering.members.map((member) => {
            const user = member.user;
            if (!user) return null;
            const me = user.id === myId;
            return (
              <li key={user.id}>
                <button
                  type="button"
                  disabled={me}
                  onClick={() => push({ kind: "friend", userId: user.id })}
                  className="flex w-full items-center gap-3 rounded-2xl px-2 py-2 text-left enabled:hover:bg-surface-2"
                >
                  <Avatar user={user} size="md" outlined />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-bold">
                      {me ? t("you") : user.displayName}
                    </span>
                    <span className="block truncate text-xs text-ink-soft">@{user.handle}</span>
                  </span>
                  <span className="shrink-0 text-xs font-bold text-ink-soft">
                    {t("gathering.here", {
                      duration: compactDuration(now - toMillis(member.arrivedAt)),
                    })}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </SheetSection>
      {others.length > 0 && (
        <SheetSection title={t("gathering.reactAll")}>
          <ReactionBar userIds={others.map((member) => member.user?.id ?? "")} />
        </SheetSection>
      )}
    </MapSheet>
  );
}
