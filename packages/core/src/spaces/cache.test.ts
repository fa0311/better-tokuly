import { create } from "@bufbuild/protobuf";
import { QueryClient } from "@tanstack/query-core";
import { spaces } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { applySpacesEvent } from "./cache";
import { spacesKeys } from "./keys";
import type { HangoutBoard } from "./queries";

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
