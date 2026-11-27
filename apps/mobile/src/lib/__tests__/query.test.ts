import { focusManager, onlineManager, QueryObserver } from "@tanstack/react-query";
import { addNetworkStateListener } from "expo-network";
import { AppState, type AppStateStatus } from "react-native";
import { createQueryClient } from "../query";

/** What the phone tells the app, as the app listens for it. */
function appStateListener(): (state: AppStateStatus) => void {
  const calls = jest.mocked(AppState.addEventListener).mock.calls;
  const call = calls.findLast(([event]) => event === "change");
  if (!call) throw new Error("nobody listens to the app state");
  return call[1] as (state: AppStateStatus) => void;
}
function networkListener(): (state: { isConnected?: boolean }) => void {
  const call = jest.mocked(addNetworkStateListener).mock.calls.at(-1);
  if (!call) throw new Error("nobody listens to the network");
  return call[0] as (state: { isConnected?: boolean }) => void;
}

jest.spyOn(AppState, "addEventListener");

describe("the query cache follows the phone", () => {
  it("reads what is on screen again when the app is back in front", async () => {
    // As the account's scope holds it.
    const queryClient = createQueryClient();
    queryClient.mount();
    let reads = 0;
    const observer = new QueryObserver(queryClient, {
      queryKey: ["on-screen"],
      queryFn: () => ++reads,
      staleTime: 0,
    });
    const off = observer.subscribe(() => {});
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(reads).toBe(1);

    appStateListener()("background");
    expect(focusManager.isFocused()).toBe(false);
    appStateListener()("active");
    expect(focusManager.isFocused()).toBe(true);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(reads).toBe(2);
    off();
    queryClient.unmount();
    queryClient.clear();
  });

  it("knows when the phone is offline, and when it is back", () => {
    const queryClient = createQueryClient();
    const off = new QueryObserver(queryClient, { queryKey: ["any"], queryFn: () => 1 }).subscribe(
      () => {},
    );
    networkListener()({ isConnected: false });
    expect(onlineManager.isOnline()).toBe(false);
    networkListener()({ isConnected: true });
    expect(onlineManager.isOnline()).toBe(true);
    // Not known yet: online.
    networkListener()({ isConnected: false });
    networkListener()({});
    expect(onlineManager.isOnline()).toBe(true);
    off();
    queryClient.clear();
  });
});
