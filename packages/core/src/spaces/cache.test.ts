import { create } from "@bufbuild/protobuf";
import { QueryClient } from "@tanstack/query-core";
import { spaces } from "@tokuly/api";
import { describe, expect, it, vi } from "vitest";
import { applySpacesEvent } from "./cache";
import { spacesKeys } from "./keys";
import { type HangoutBoard, spacesQueries } from "./queries";

const room = create(spaces.SpaceSchema, { id: "s1", state: spaces.SpaceState.LIVE });

describe("pushed events in the cache", () => {
  it("a head count reaches the room's page and every list that shows it", () => {
    const queryClient = new QueryClient();
    const listKey = spacesKeys.live(spaces.LiveScope.UNSPECIFIED, "", "");
    queryClient.setQueryData(spacesKeys.space("s1"), room);
    queryClient.setQueryData(listKey, {
      pages: [{ spaces: [room], nextCursor: "" }],
      pageParams: [""],
    });
    applySpacesEvent(
      queryClient,
      {
        case: "counts",
        value: create(spaces.SpaceCountsSchema, { spaceId: "s1", listenerCount: 7 }),
      },
      "me",
    );
    expect(queryClient.getQueryData<spaces.Space>(spacesKeys.space("s1"))?.listenerCount).toBe(7);
    const list = queryClient.getQueryData<{ pages: { spaces: spaces.Space[] }[] }>(listKey);
    expect(list?.pages[0]?.spaces[0]?.listenerCount).toBe(7);
  });

  it("a removed hangout leaves its page and the board", () => {
    const queryClient = new QueryClient();
    const hangout = create(spaces.HangoutSchema, { id: "h1" });
    queryClient.setQueryData(spacesKeys.hangout("h1"), hangout);
    queryClient.setQueryData<HangoutBoard>(spacesKeys.hangouts, {
      hangouts: [hangout],
      invitations: [],
    });
    applySpacesEvent(
      queryClient,
      { case: "hangoutRemoved", value: create(spaces.HangoutRemovedSchema, { hangoutId: "h1" }) },
      "me",
    );
    expect(queryClient.getQueryData(spacesKeys.hangout("h1"))).toBeNull();
    expect(queryClient.getQueryData<HangoutBoard>(spacesKeys.hangouts)?.hangouts).toEqual([]);
  });

  it("a banned viewer no longer has the room", () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData(spacesKeys.space("s1"), room);
    applySpacesEvent(
      queryClient,
      {
        case: "participantLeft",
        value: create(spaces.SpaceParticipantLeftSchema, {
          spaceId: "s1",
          userId: "me",
          reason: spaces.SpaceLeaveReason.BANNED,
        }),
      },
      "me",
    );
    expect(queryClient.getQueryData(spacesKeys.space("s1"))).toBeNull();
  });
});

/** Answers the test hands out, one per request, in the order the requests were made. */
function server<T>() {
  const pending: ((answer: T) => void)[] = [];
  return {
    request: () => new Promise<T>((resolve) => pending.push(resolve)),
    answer: (request: number, answer: T) => pending[request]?.(answer),
  };
}

const removed = (hangoutId: string): spaces.SpacesEvent["event"] => ({
  case: "hangoutRemoved",
  value: create(spaces.HangoutRemovedSchema, { hangoutId }),
});

describe("hangouts pushed during a read", () => {
  // Who is inside: what a person taken out of the hangout must no longer see.
  const hangout = create(spaces.HangoutSchema, {
    id: "h1",
    insideCount: 1,
    members: [
      {
        user: { id: "friend" },
        state: spaces.HangoutMemberState.MEMBER,
        inside: true,
      },
    ],
  });

  it("a hangout the person was taken out of stays gone when an older read lands", async () => {
    const queryClient = new QueryClient();
    const api = server<spaces.GetHangoutResponse>();
    const client = () => ({ getHangout: api.request }) as never;
    const loading = queryClient.fetchQuery(spacesQueries.hangout(client, "h1"));
    applySpacesEvent(queryClient, removed("h1"), "me");
    // The answer was made before the removal.
    api.answer(0, create(spaces.GetHangoutResponseSchema, { hangout }));
    await loading;
    expect(queryClient.getQueryData(spacesKeys.hangout("h1"))).toBeNull();
  });

  it("the board read before the removal does not bring the hangout back", async () => {
    const queryClient = new QueryClient();
    const api = server<spaces.ListHangoutsResponse>();
    const client = () => ({ listHangouts: api.request }) as never;
    const loading = queryClient.fetchQuery(spacesQueries.hangouts(client));
    applySpacesEvent(queryClient, removed("h1"), "me");
    api.answer(0, create(spaces.ListHangoutsResponseSchema, { hangouts: [hangout] }));
    await loading;
    expect(queryClient.getQueryData<HangoutBoard>(spacesKeys.hangouts)).toEqual({
      hangouts: [],
      invitations: [],
    });
  });

  it("someone leaving the voice room during a read is not inside when it lands", async () => {
    const queryClient = new QueryClient();
    const api = server<spaces.GetHangoutResponse>();
    const client = () => ({ getHangout: api.request }) as never;
    const loading = queryClient.fetchQuery(spacesQueries.hangout(client, "h1"));
    applySpacesEvent(
      queryClient,
      {
        case: "hangoutPresence",
        value: create(spaces.HangoutPresenceSchema, {
          hangoutId: "h1",
          user: { id: "friend" },
          inside: false,
          insideCount: 0,
        }),
      },
      "me",
    );
    api.answer(0, create(spaces.GetHangoutResponseSchema, { hangout }));
    await loading;
    const read = queryClient.getQueryData<spaces.Hangout>(spacesKeys.hangout("h1"));
    expect(read?.insideCount).toBe(0);
    expect(read?.members.filter((member) => member.inside)).toEqual([]);
  });

  it("someone entering the voice room during a read is inside since the push", async () => {
    vi.useFakeTimers({ now: new Date("2026-11-29T10:00:00Z") });
    try {
      const queryClient = new QueryClient();
      const api = server<spaces.GetHangoutResponse>();
      const client = () => ({ getHangout: api.request }) as never;
      const loading = queryClient.fetchQuery(spacesQueries.hangout(client, "h1"));
      const outside = create(spaces.HangoutSchema, {
        id: "h1",
        members: [{ user: { id: "friend" }, state: spaces.HangoutMemberState.MEMBER }],
      });
      applySpacesEvent(
        queryClient,
        {
          case: "hangoutPresence",
          value: create(spaces.HangoutPresenceSchema, {
            hangoutId: "h1",
            user: { id: "friend" },
            inside: true,
            insideCount: 1,
          }),
        },
        "me",
      );
      // The answer lands a minute later; the push is replayed on it then.
      vi.setSystemTime(new Date("2026-11-29T10:01:00Z"));
      api.answer(0, create(spaces.GetHangoutResponseSchema, { hangout: outside }));
      await loading;
      const read = queryClient.getQueryData<spaces.Hangout>(spacesKeys.hangout("h1"));
      const [member] = read?.members ?? [];
      expect(member?.inside).toBe(true);
      expect(Number(member?.insideSince?.seconds)).toBe(Date.parse("2026-11-29T10:00:00Z") / 1000);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("rooms pushed during a read", () => {
  const speaker = (id: string) =>
    create(spaces.SpaceParticipantSchema, { user: { id }, role: spaces.SpaceRole.SPEAKER });
  const live = create(spaces.SpaceSchema, {
    id: "s1",
    state: spaces.SpaceState.LIVE,
    stage: [speaker("host"), speaker("guest")],
  });
  const ended: spaces.SpacesEvent["event"] = {
    case: "spaceUpdated",
    value: create(spaces.SpaceUpdatedSchema, {
      space: { ...live, state: spaces.SpaceState.ENDED, stage: [] },
    }),
  };
  const guestLeft: spaces.SpacesEvent["event"] = {
    case: "participantLeft",
    value: create(spaces.SpaceParticipantLeftSchema, { spaceId: "s1", userId: "guest" }),
  };

  it("an ended room does not show LIVE again when an older read lands", async () => {
    const queryClient = new QueryClient();
    const api = server<spaces.GetSpaceResponse>();
    const client = () => ({ getSpace: api.request }) as never;
    // The room's topic took effect: it is read again; it ends before the answer, made earlier, lands.
    queryClient.setQueryData(spacesKeys.space("s1"), live);
    const loading = queryClient.fetchQuery({ ...spacesQueries.space(client, "s1"), staleTime: 0 });
    applySpacesEvent(queryClient, ended, "me");
    api.answer(0, create(spaces.GetSpaceResponseSchema, { space: live }));
    await loading;
    expect(queryClient.getQueryData<spaces.Space>(spacesKeys.space("s1"))?.state).toBe(
      spaces.SpaceState.ENDED,
    );
  });

  it("an ended room does not come back LIVE on a list read before", async () => {
    const queryClient = new QueryClient();
    const api = server<spaces.ListLiveSpacesResponse>();
    const client = () => ({ listLiveSpaces: api.request }) as never;
    const query = spacesQueries.live(client, spaces.LiveScope.UNSPECIFIED);
    const loading = queryClient.fetchInfiniteQuery(query);
    applySpacesEvent(queryClient, ended, "me");
    // The read made before the end lands; the lists are read again after it, too.
    api.answer(0, create(spaces.ListLiveSpacesResponseSchema, { spaces: [live] }));
    const endedRoom = { ...live, state: spaces.SpaceState.ENDED, stage: [] };
    api.answer(1, create(spaces.ListLiveSpacesResponseSchema, { spaces: [endedRoom] }));
    await loading;
    const list = queryClient.getQueryData<{ pages: { spaces: spaces.Space[] }[] }>(query.queryKey);
    expect(list?.pages[0]?.spaces[0]?.state).toBe(spaces.SpaceState.ENDED);
  });

  it("someone who left is not back on stage, nor among the people inside", async () => {
    const queryClient = new QueryClient();
    const rooms = server<spaces.GetSpaceResponse>();
    const people = server<spaces.ListParticipantsResponse>();
    const client = () => ({ getSpace: rooms.request, listParticipants: people.request }) as never;
    const room = queryClient.fetchQuery(spacesQueries.space(client, "s1"));
    const inside = spacesQueries.participants(client, "s1", spaces.ParticipantFilter.UNSPECIFIED);
    const list = queryClient.fetchInfiniteQuery(inside);
    applySpacesEvent(queryClient, guestLeft, "me");
    rooms.answer(0, create(spaces.GetSpaceResponseSchema, { space: live }));
    people.answer(0, create(spaces.ListParticipantsResponseSchema, { participants: live.stage }));
    await Promise.all([room, list]);
    const stage = queryClient.getQueryData<spaces.Space>(spacesKeys.space("s1"))?.stage;
    expect(stage?.map((entry) => entry.user?.id)).toEqual(["host"]);
    const listed = queryClient.getQueryData<{
      pages: { participants: spaces.SpaceParticipant[] }[];
    }>(inside.queryKey);
    expect(listed?.pages[0]?.participants.map((entry) => entry.user?.id)).toEqual(["host"]);
  });
});
