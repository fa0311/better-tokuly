import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen } from "@testing-library/react-native";
import { create, fromDate, plans } from "@tokuly/api";
import type { ReactNode } from "react";
import { Linking } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { useSessionStore } from "#/features/session/store";
import { api } from "#/lib/api";
import { PlanScreen } from "../plan-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({
  router: { push: jest.fn(), replace: jest.fn(), back: jest.fn() },
  Stack: { Screen: () => null },
}));
// The map is the device's.
jest.mock("@maplibre/maplibre-react-native", () => {
  const { View } = require("react-native");
  const Stub = ({ children }: { children?: ReactNode }) =>
    require("react").createElement(View, null, children);
  return { Map: Stub, Camera: () => null, Marker: Stub };
});

const ME = { id: "me", handle: "me", displayName: "わたし" };
const YUI = { id: "yui", handle: "yui", displayName: "ゆい" };

const PLAN = create(plans.PlanSchema, {
  id: "p1",
  title: "放課後カラオケ",
  emoji: "🎤",
  status: plans.PlanStatus.ACTIVE,
  // 18:00 on Christmas Eve in Tokyo.
  time: {
    start: { case: "startAt", value: fromDate(new Date("2026-12-24T09:00:00Z")) },
    timeZone: "Asia/Tokyo",
  },
  place: { name: "駅前カラオケ", address: "渋谷1-2-3", location: { lat: 35.66, lng: 139.7 } },
  remindDayBefore: true,
  remindMinutesBefore: 60,
  myRole: plans.PlanRole.GUEST,
  myRsvp: plans.Rsvp.INVITED,
  counts: { going: 1, invited: 1 },
  participants: [
    { user: YUI, role: plans.PlanRole.HOST, rsvp: plans.Rsvp.GOING, comment: "たのしみ！" },
    { user: ME, role: plans.PlanRole.GUEST, rsvp: plans.Rsvp.INVITED },
  ],
});

/** The API of the account in use: the plan, the friends, and every request it was sent. */
function signIn() {
  const sent: { method: string; request: unknown }[] = [];
  const service = {
    getPlan: async () => ({ plan: PLAN }),
    respondToPlan: async (request: { rsvp: plans.Rsvp }) => {
      sent.push({ method: "respondToPlan", request });
      return { plan: { ...PLAN, myRsvp: request.rsvp }, rsvp: request.rsvp };
    },
  };
  jest.mocked(api).mockReturnValue({ service: () => service } as never);
  useSessionStore.setState({
    state: {
      status: "signedIn",
      sessionId: "s1",
      me: { user: ME, timeZone: "Asia/Tokyo" },
    } as never,
  });
  return sent;
}

async function show(children: ReactNode) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("PlanScreen", () => {
  it("shows when and where on the account's clock, and sends the answer as the account", async () => {
    const sent = signIn();
    await show(<PlanScreen planId="p1" />);
    expect(await screen.findByTestId("plan-time")).toHaveTextContent("12月24日(木) 18:00〜");
    expect(screen.getByTestId("plan-reminders")).toHaveTextContent(
      "前の日にお知らせ · 60分前にお知らせ",
    );
    expect(screen.getByText("「たのしみ！」")).toBeOnTheScreen();
    expect(screen.getByText("行く？ 返事してね！")).toBeOnTheScreen();

    await act(() => fireEvent.press(screen.getByTestId("plan-rsvp-going")));
    expect(sent).toEqual([
      {
        method: "respondToPlan",
        request: { planId: "p1", rsvp: plans.Rsvp.GOING, comment: "" },
      },
    ]);
    expect(await screen.findByText("あなたの返事")).toBeOnTheScreen();
  });

  it("opens the place in the map app", async () => {
    signIn();
    const openURL = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
    await show(<PlanScreen planId="p1" />);
    await act(async () => fireEvent.press(await screen.findByTestId("plan-place")));
    expect(openURL).toHaveBeenCalledWith(
      expect.stringMatching(/^(geo:35\.66,139\.7\?q=|maps:\?ll=35\.66,139\.7&q=)/),
    );
  });
});
