import { fireEvent, render, screen, within } from "@testing-library/react-native";
import { router } from "expo-router";
import Me from "../../../../app/(app)/(tabs)/me";

jest.mock("expo-router", () => ({ router: { push: jest.fn() } }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({}) }));
jest.mock("#/features/profile/profile-screen", () => ({
  ProfileScreen: ({ footer }: { footer: React.ReactNode }) => footer,
}));
jest.mock("#/features/session/store", () => ({
  useMe: () => ({ user: { handle: "me" } }),
}));
jest.mock("#/seams/badges", () => ({ useBadges: () => ({ friends: 2 }) }));
jest.mock("#/seams/hub", () => ({
  HUB_ENTRIES: [
    { id: "friends", href: "/friends", label: () => "ともだち", badge: "friends" },
    { id: "settings", href: "/settings", label: () => "設定" },
  ],
}));

test("the profile hub shows each Row and opens its destination", async () => {
  await render(<Me />);

  const friends = screen.getByTestId("me-friends");
  expect(within(friends).getByText("ともだち")).toBeOnTheScreen();
  expect(within(friends).getByText("2")).toBeOnTheScreen();
  expect(within(screen.getByTestId("me-settings")).getByText("設定")).toBeOnTheScreen();

  await fireEvent.press(friends);
  expect(router.push).toHaveBeenLastCalledWith("/friends");
  await fireEvent.press(screen.getByTestId("me-settings"));
  expect(router.push).toHaveBeenLastCalledWith("/settings");
});
