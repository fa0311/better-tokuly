import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type common, errorReason, identity, social } from "@tokuly/api";
import { friendKeys, friendQueries, requestNote } from "@tokuly/core/friends";
import { Check, ChevronRight, Copy, Radar, Star, UserPlus, X } from "lucide-react";
import QRCode from "qrcode";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import {
  type ClaimProblem,
  claimProblem,
  PRECISE_ENOUGH_METERS,
  reportFreshFix,
} from "#/features/map/own-location";
import { api, profileClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { useLocations } from "#/stores/locations";
import { useFollowRequestCount } from "./follow-data";
import { FollowRequests } from "./follow-requests";
import { FollowSuggestions } from "./follow-suggestions";
import { RowsSkeleton, SearchInput, SectionTitle, UserRow } from "./parts";

const friendClient = () => api().service(social.FriendService);

type Tab = "friends" | "requests" | "find" | "invite";

/** Friends hub: your friends, pending requests, ways to find people, and your invite. */
export function FriendsScreen() {
  const { t } = useTranslation("friends");
  const [tab, setTab] = useState<Tab>("friends");
  const requests = useQuery(friendQueries.incoming(friendClient));
  const followRequests = useFollowRequestCount();

  return (
    <Page title={t("title")}>
      <Tabs<Tab>
        className="mb-4"
        value={tab}
        onChange={setTab}
        items={[
          { value: "friends", label: t("tabs.friends") },
          {
            value: "requests",
            label: t("tabs.requests"),
            badge: (requests.data?.incomingCount ?? 0) + followRequests,
          },
          { value: "find", label: t("tabs.find") },
          { value: "invite", label: t("tabs.invite") },
        ]}
      />
      {tab === "friends" && <FriendList />}
      {tab === "requests" && <Requests />}
      {tab === "find" && <Find />}
      {tab === "invite" && <Invite />}
    </Page>
  );
}

/** The way to the viewer's circles (親しい友達 and custom lists) from the friends list. */
function CirclesLink() {
  const { t } = useTranslation("circles");
  return (
    <Link
      to="/circles"
      className="pressable sticker-sm flex items-center gap-3 rounded-2xl bg-surface px-3 py-2.5"
    >
      <span className="sticker-sm grid size-9 place-items-center rounded-full bg-mint text-[#17122b]">
        <Star className="size-5 fill-current" aria-hidden />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block font-black">{t("tile")}</span>
        <span className="block truncate text-xs text-ink-soft">{t("tileHint")}</span>
      </span>
      <ChevronRight className="size-5 text-ink-faint" aria-hidden />
    </Link>
  );
}

function FriendList() {
  const { t } = useTranslation("friends");
  const [query, setQuery] = useState("");
  const { data, isPending } = useQuery(friendQueries.list(friendClient, query));
  const online = useLocations((s) => s.online);

  return (
    <div className="flex flex-col gap-2">
      <SearchInput value={query} onChange={setQuery} placeholder={t("search.placeholder")} />
      <CirclesLink />
      {isPending ? (
        <RowsSkeleton />
      ) : data && data.friends.length > 0 ? (
        <>
          <p className="px-3 pt-1 text-xs font-bold text-ink-faint">
            {t("list.count", { count: data.total })}
          </p>
          {data.friends.map(
            (friend) =>
              friend.user && (
                <UserRow
                  key={friend.user.id}
                  user={friend.user}
                  subtitle={
                    friend.status?.text
                      ? `${friend.status.emoji} ${friend.status.text}`
                      : `@${friend.user.handle}`
                  }
                  trailing={
                    <span className="flex items-center gap-2 text-lg">
                      {friend.bestFriendRank > 0 && <span title="best friend">💜</span>}
                      {friend.favorite && <span title="favorite">⭐</span>}
                      {online[friend.user.id] && (
                        <span className="size-2.5 rounded-full bg-success" />
                      )}
                    </span>
                  }
                />
              ),
          )}
        </>
      ) : (
        <EmptyState
          emoji="🫡"
          title={query ? t("search.empty") : t("list.empty")}
          body={query ? undefined : t("list.emptyBody")}
        />
      )}
    </div>
  );
}

function Requests() {
  const { t, i18n } = useTranslation("friends");
  const queryClient = useQueryClient();
  const incoming = useQuery(friendQueries.incoming(friendClient));
  const outgoing = useQuery(friendQueries.outgoing(friendClient));
  const refresh = () => queryClient.invalidateQueries({ queryKey: friendKeys.all });

  const respond = useMutation({
    mutationFn: (input: { requestId: string; accept: boolean }) =>
      friendClient().respondFriendRequest(input),
    onSuccess: (_, input) => {
      if (input.accept) toast.success(t("toast.accepted"));
      return refresh();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  const cancel = useMutation({
    mutationFn: (requestId: string) => friendClient().cancelFriendRequest({ requestId }),
    onSuccess: () => {
      toast.success(t("toast.cancelled"));
      return refresh();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <div className="flex flex-col gap-6">
      <section>
        <SectionTitle>{t("requests.incoming")}</SectionTitle>
        {incoming.isPending ? (
          <RowsSkeleton />
        ) : incoming.data?.requests.length ? (
          incoming.data.requests.map(
            (request) =>
              request.from && (
                <UserRow
                  key={request.id}
                  user={request.from}
                  subtitle={
                    // Whom both know is what helps decide: it may take a second line.
                    <span className="line-clamp-2 whitespace-normal">
                      {requestNote(i18n, request) ?? `@${request.from.handle}`}
                    </span>
                  }
                  trailing={
                    <span className="flex gap-2">
                      <Button
                        size="sm"
                        variant="lime"
                        icon={<Check className="size-4" />}
                        onClick={() => respond.mutate({ requestId: request.id, accept: true })}
                      >
                        {t("requests.accept")}
                      </Button>
                      <Button
                        size="icon-sm"
                        variant="ghost"
                        aria-label={t("requests.decline")}
                        onClick={() => respond.mutate({ requestId: request.id, accept: false })}
                      >
                        <X className="size-4" />
                      </Button>
                    </span>
                  }
                />
              ),
          )
        ) : (
          <p className="px-3 py-4 text-sm text-ink-faint">{t("requests.emptyIncoming")}</p>
        )}
      </section>
      <section>
        <SectionTitle>{t("requests.outgoing")}</SectionTitle>
        {outgoing.isPending ? (
          <RowsSkeleton />
        ) : outgoing.data?.requests.length ? (
          outgoing.data.requests.map(
            (request) =>
              request.to && (
                <UserRow
                  key={request.id}
                  user={request.to}
                  trailing={
                    <Button size="sm" variant="secondary" onClick={() => cancel.mutate(request.id)}>
                      {t("requests.cancel")}
                    </Button>
                  }
                />
              ),
          )
        ) : (
          <p className="px-3 py-4 text-sm text-ink-faint">{t("requests.emptyOutgoing")}</p>
        )}
      </section>
      <div className="border-t-2 border-line pt-5">
        <h2 className="px-3 pb-3 text-base font-black">{t("requests.followTitle")}</h2>
        <FollowRequests />
      </div>
    </div>
  );
}

/** The add/requested/friends control next to a person. */
function RelationshipButton({
  user,
  relationship,
  source,
}: {
  user: common.UserSummary;
  relationship: identity.Relationship;
  source: social.FriendSource;
}) {
  const { t } = useTranslation("friends");
  const queryClient = useQueryClient();
  const [current, setCurrent] = useState(relationship);
  const add = useMutation({
    mutationFn: () =>
      friendClient().sendFriendRequest({ target: { case: "userId", value: user.id }, source }),
    onSuccess: (response) => {
      setCurrent(response.relationship);
      toast.success(
        response.relationship === identity.Relationship.FRIENDS
          ? t("toast.accepted")
          : t("toast.sent"),
      );
      return queryClient.invalidateQueries({ queryKey: friendKeys.all });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  if (current === identity.Relationship.SELF) return null;
  if (current === identity.Relationship.FRIENDS)
    return (
      <span className="rounded-full bg-surface-2 px-3 py-1 text-xs font-bold text-ink-soft">
        {t("actions.friends")}
      </span>
    );
  if (current === identity.Relationship.REQUEST_SENT)
    return (
      <span className="rounded-full bg-surface-2 px-3 py-1 text-xs font-bold text-ink-soft">
        {t("actions.requested")}
      </span>
    );
  return (
    <Button
      size="sm"
      icon={<UserPlus className="size-4" />}
      loading={add.isPending}
      onClick={() => add.mutate()}
    >
      {current === identity.Relationship.REQUEST_RECEIVED ? t("actions.accept") : t("actions.add")}
    </Button>
  );
}

function Find() {
  const { t } = useTranslation("friends");
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(query.trim()), 300);
    return () => clearTimeout(timer);
  }, [query]);

  const results = useQuery({
    queryKey: ["identity", "search", debounced],
    queryFn: () => profileClient().searchUsers({ query: debounced }),
    enabled: debounced.length > 0,
  });
  const suggestions = useQuery(friendQueries.suggestions(friendClient));

  return (
    <div className="flex flex-col gap-4">
      <SearchInput value={query} onChange={setQuery} placeholder={t("search.placeholder")} />
      {debounced ? (
        results.isPending ? (
          <RowsSkeleton />
        ) : results.data?.results.length ? (
          <div>
            {results.data.results.map(
              (result) =>
                result.user && (
                  <UserRow
                    key={result.user.id}
                    user={result.user}
                    subtitle={
                      result.mutualFriendCount > 0
                        ? t("requests.mutual", { count: result.mutualFriendCount })
                        : undefined
                    }
                    trailing={
                      <RelationshipButton
                        user={result.user}
                        relationship={result.relationship}
                        source={social.FriendSource.HANDLE}
                      />
                    }
                  />
                ),
            )}
          </div>
        ) : (
          <EmptyState emoji="🔍" title={t("search.empty")} />
        )
      ) : (
        <>
          <Nearby />
          {suggestions.data && suggestions.data.suggestions.length > 0 && (
            <section>
              <SectionTitle>{t("suggestions.title")}</SectionTitle>
              {suggestions.data.suggestions.map(
                (suggestion) =>
                  suggestion.user && (
                    <UserRow
                      key={suggestion.user.id}
                      user={suggestion.user}
                      subtitle={
                        suggestion.mutualFriendCount > 0
                          ? t("suggestions.mutual", { count: suggestion.mutualFriendCount })
                          : undefined
                      }
                      trailing={
                        <RelationshipButton
                          user={suggestion.user}
                          relationship={identity.Relationship.NONE}
                          source={social.FriendSource.SUGGESTION}
                        />
                      }
                    />
                  ),
              )}
            </section>
          )}
          <FollowSuggestions />
        </>
      )}
    </div>
  );
}

/**
 * Nearby discovery: while this is running we are visible to — and see —
 * other people with it open within about 50 m.
 */
function Nearby() {
  const { t } = useTranslation("friends");
  const { t: tc } = useTranslation("common");
  const { t: te } = useTranslation("errors");
  const own = useLocations((s) => s.own);
  const [active, setActive] = useState(false);
  const [starting, setStarting] = useState(false);
  /** The server could not confirm we are where we said: shown with a retry. */
  const [unverified, setUnverified] = useState<ClaimProblem | null>(null);
  const [users, setUsers] = useState<social.NearbyUser[]>([]);

  // A session opens only where our own latest fix puts us: send a fresh one first.
  const start = async () => {
    setStarting(true);
    setUnverified(null);
    try {
      const fix = await reportFreshFix();
      // Approximate location (an OS setting) is refused anyway: say so without asking.
      if (fix && fix.accuracyMeters > PRECISE_ENOUGH_METERS) {
        setUnverified("LOCATION_TOO_COARSE");
        return;
      }
    } finally {
      setStarting(false);
    }
    setActive(true);
  };

  const lat = own?.lat;
  const lng = own?.lng;
  const accuracy = own?.accuracyMeters;
  useEffect(() => {
    if (!active || lat === undefined || lng === undefined) return;
    const abort = new AbortController();
    void (async () => {
      try {
        const stream = friendClient().discoverNearby(
          { location: { lat, lng }, accuracyMeters: accuracy ?? 0 },
          { signal: abort.signal },
        );
        for await (const update of stream) setUsers(update.users);
      } catch (error) {
        if (abort.signal.aborted) return;
        const problem = claimProblem(errorReason(error));
        if (problem) {
          setActive(false);
          setUnverified(problem);
        } else toast.error(errorMessage(error));
      }
    })();
    return () => {
      abort.abort();
      setUsers([]);
    };
    // Coordinates are rounded so tiny GPS jitter does not restart the stream.
  }, [
    active,
    lat !== undefined && Math.round(lat * 2000),
    lng !== undefined && Math.round(lng * 2000),
    accuracy !== undefined && Math.round(accuracy / 25),
  ]);

  return (
    <Card className="p-4">
      <div className="flex items-center gap-3">
        <div
          className={`sticker-sm grid size-12 place-items-center rounded-2xl bg-sky text-[#17122b] ${active ? "animate-wiggle" : ""}`}
        >
          <Radar className="size-6" />
        </div>
        <div className="min-w-0 flex-1">
          <p className="font-black">{t("nearby.title")}</p>
          <p className="text-sm text-ink-soft">
            {active ? t("nearby.lead") : !own ? t("nearby.needLocation") : ""}
          </p>
        </div>
        <Button
          size="sm"
          variant={active ? "secondary" : "primary"}
          disabled={!own || starting}
          onClick={() => (active ? setActive(false) : void start())}
        >
          {active ? t("nearby.stop") : t("nearby.start")}
        </Button>
      </div>
      {unverified && !active && (
        <div
          role="alert"
          className="mt-3 flex items-center gap-2 border-t-2 border-line pt-2 text-sm"
          data-testid="nearby-unverified"
        >
          <p className="min-w-0 flex-1">{te(unverified)}</p>
          <Button size="sm" variant="secondary" disabled={starting} onClick={() => void start()}>
            {tc("actions.retry")}
          </Button>
        </div>
      )}
      {active && users.length > 0 && (
        <div className="mt-3 border-t-2 border-line pt-2">
          {users.map(
            (nearby) =>
              nearby.user && (
                <UserRow
                  key={nearby.user.id}
                  user={nearby.user}
                  subtitle={t("nearby.distance", { meters: Math.round(nearby.distanceMeters) })}
                  trailing={
                    <RelationshipButton
                      user={nearby.user}
                      relationship={nearby.relationship}
                      source={social.FriendSource.NEARBY}
                    />
                  }
                />
              ),
          )}
        </div>
      )}
    </Card>
  );
}

function Invite() {
  const { t } = useTranslation(["friends", "common"]);
  const { data } = useQuery(friendQueries.invite(friendClient));
  const [qr, setQr] = useState<string | null>(null);
  const url = data?.invite?.url;

  useEffect(() => {
    if (!url) return;
    let cancelled = false;
    QRCode.toDataURL(url, {
      margin: 1,
      width: 480,
      color: { dark: "#17122b", light: "#ffffff" },
    }).then((dataUrl) => {
      if (!cancelled) setQr(dataUrl);
    });
    return () => {
      cancelled = true;
    };
  }, [url]);

  const copy = async () => {
    if (!url) return;
    await navigator.clipboard.writeText(url);
    toast.success(t("common:actions.copied"));
  };
  const share = async () => {
    if (!url) return;
    if (navigator.share)
      await navigator
        .share({ title: "Tokuly", text: t("friends:invite.share"), url })
        .catch(() => {});
    else await copy();
  };

  return (
    <Card className="flex flex-col items-center gap-4 text-center">
      <h2 className="text-xl font-black">{t("friends:invite.title")}</h2>
      <p className="max-w-xs text-sm text-ink-soft">{t("friends:invite.lead")}</p>
      <div className="sticker grid size-60 place-items-center rounded-3xl bg-white p-3">
        {qr ? <img src={qr} alt="QR" className="size-full" /> : <Skeleton className="size-full" />}
      </div>
      {url && (
        <p className="max-w-full truncate rounded-full bg-surface-2 px-4 py-2 text-sm font-bold">
          {url}
        </p>
      )}
      <div className="flex gap-2">
        <Button
          variant="secondary"
          icon={<Copy className="size-4" />}
          onClick={copy}
          disabled={!url}
        >
          {t("friends:invite.copy")}
        </Button>
        <Button onClick={share} disabled={!url}>
          {t("common:actions.share")}
        </Button>
      </div>
      {data?.invite && data.invite.introCount > 0 && (
        <p className="text-sm font-bold text-brand">
          {t("friends:invite.intros", { count: data.invite.introCount })}
        </p>
      )}
    </Card>
  );
}
