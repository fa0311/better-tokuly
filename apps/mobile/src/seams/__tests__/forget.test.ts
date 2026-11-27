import { create, geo } from "@tokuly/api";
import { useLive } from "#/features/chat/live";
import { useLocations } from "#/features/map/locations-store";
import { useAudioSession } from "#/features/spaces/session";
import { runForget } from "../lifecycle";

/** What the account that leaves had in memory, outside the signed-in tree. */
function asTheLeavingAccount(): void {
  useLocations
    .getState()
    .setSnapshot([create(geo.FriendLocationSchema, { userId: "friend-of-a" })]);
  useLocations.getState().setOnline("friend-of-a", true);
  useLocations.getState().setOwn({ latitude: 35.66, longitude: 139.7 } as never);
  useLive.getState().setTyping("conversation-of-a", "friend-of-a", true);
  useAudioSession.setState({
    active: { kind: "space", id: "room-of-a" } as never,
    exit: { kind: "space", id: "room-of-a", reason: "removed" } as never,
  });
}

describe("an account that left", () => {
  it("leaves nothing in memory for the next one", () => {
    asTheLeavingAccount();
    runForget();

    expect(useLocations.getState()).toMatchObject({ friends: {}, online: {}, own: null });
    expect(useLive.getState().typing).toEqual({});
    expect(useAudioSession.getState()).toMatchObject({ active: null, exit: null });
  });
});
