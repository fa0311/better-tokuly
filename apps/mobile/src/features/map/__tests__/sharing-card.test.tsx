import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import * as Location from "expo-location";
import * as SecureStore from "expo-secure-store";
import { choiceKey, useSharing } from "../sharing";
import { SharingCard } from "../sharing-card";

jest.mock("expo-location", () => ({
  hasServicesEnabledAsync: jest.fn(async () => true),
  getForegroundPermissionsAsync: jest.fn(),
  requestForegroundPermissionsAsync: jest.fn(),
  getBackgroundPermissionsAsync: jest.fn(async () => ({ granted: false, canAskAgain: true })),
  requestBackgroundPermissionsAsync: jest.fn(),
}));

const permission = (granted: boolean) => ({
  granted,
  canAskAgain: !granted,
  status: granted ? "granted" : "undetermined",
  android: { accuracy: granted ? "fine" : "none" },
});

const NOT_ASKED = {
  servicesEnabled: true,
  foreground: "undetermined" as const,
  background: false,
  approximate: false,
};

beforeEach(() => {
  useSharing.setState({
    userId: "me",
    loaded: true,
    choice: null,
    access: NOT_ASKED,
    stopOwed: false,
  });
});

describe("SharingCard", () => {
  it("asks for consent first, explaining what the OS will ask", async () => {
    await render(<SharingCard />);
    expect(screen.getByText("いまどこ？をシェアしよう！")).toBeOnTheScreen();
    expect(screen.getByText(/アプリの使用中は許可/)).toBeOnTheScreen();
    expect(Location.requestForegroundPermissionsAsync).not.toHaveBeenCalled();
  });

  it("turns sharing on for this account once the OS allows it, then offers 'always'", async () => {
    jest
      .mocked(Location.requestForegroundPermissionsAsync)
      .mockResolvedValue(permission(true) as never);
    jest
      .mocked(Location.getForegroundPermissionsAsync)
      .mockResolvedValue(permission(true) as never);
    await render(<SharingCard />);
    await fireEvent.press(screen.getByTestId("sharing-turn-on"));
    await waitFor(() => expect(useSharing.getState().choice).toBe("on"));
    expect(SecureStore.setItemAsync).toHaveBeenCalledWith(choiceKey("me"), "on", expect.anything());
    expect(await screen.findByTestId("sharing-always")).toBeOnTheScreen();
  });

  it("stays undecided when the OS prompt is refused", async () => {
    jest
      .mocked(Location.requestForegroundPermissionsAsync)
      .mockResolvedValue(permission(false) as never);
    jest
      .mocked(Location.getForegroundPermissionsAsync)
      .mockResolvedValue({ ...permission(false), canAskAgain: false } as never);
    await render(<SharingCard />);
    await fireEvent.press(screen.getByTestId("sharing-turn-on"));
    await waitFor(() => expect(useSharing.getState().access?.foreground).toBe("denied"));
    expect(useSharing.getState().choice).toBeNull();
  });

  it("sends to the settings app when the permission was taken away while sharing", async () => {
    useSharing.setState({ choice: "on", access: { ...NOT_ASKED, foreground: "denied" } });
    await render(<SharingCard />);
    expect(screen.getByText("位置情報がオフになってるよ")).toBeOnTheScreen();
    expect(screen.getByText("設定をひらく")).toBeOnTheScreen();
  });
});
