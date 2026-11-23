import { Outlet } from "@tanstack/react-router";
import { errorReason } from "@tokuly/api";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { EmptyState, Page } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { errorMessage } from "#/lib/errors";
import { realtime } from "#/lib/realtime";
import { useMe } from "#/stores/session";
import { NS, useCommunity } from "./api";
import { CommunityContext } from "./context";
import { FrontDoor } from "./front-door";
import { canRead } from "./model";

/**
 * Frame of every page of one community: loads it, keeps it and its boards
 * live, and decides what the visitor gets — the pages (`<Outlet>`), or the
 * front door when the community is only for members.
 */
export function CommunityLayout({ handle }: { handle: string }) {
  const { t } = useTranslation(NS);
  const query = useCommunity(handle);
  const community = query.data;
  const roles = useMe()?.roles;
  const readable = canRead(community ?? undefined, roles);
  const communityId = community?.id;
  const refetch = query.refetch;

  // Board and calendar events of this community arrive while one of its pages is open.
  useEffect(() => {
    if (!communityId || !readable) return;
    // Removed, banned, or the community closed: read it again, which shows the front door
    // (or "not found") instead of the board.
    const unsubscribe = realtime().subscribe(`community:${communityId}`, {
      onRevoked: () => void refetch(),
    });
    realtime().start();
    return unsubscribe;
  }, [communityId, readable, refetch]);

  if (query.isPending) return <PaneSpinner />;
  if (!community) {
    const missing = !query.error || errorReason(query.error) === "COMMUNITY_NOT_FOUND";
    return (
      <Page title={t("title")} back="/communities">
        <EmptyState
          emoji={missing ? "🫥" : "🌧️"}
          title={missing ? t("notFound.title") : errorMessage(query.error, { namespace: NS })}
          body={missing ? t("notFound.body") : undefined}
          action={
            missing ? undefined : (
              <Button variant="secondary" onClick={() => void query.refetch()}>
                {t("retry")}
              </Button>
            )
          }
        />
      </Page>
    );
  }
  return (
    <CommunityContext.Provider value={community}>
      {readable ? <Outlet /> : <FrontDoor />}
    </CommunityContext.Provider>
  );
}
