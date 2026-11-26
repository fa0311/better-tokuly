import { Alert } from "react-native";
import { useSessionStore } from "#/features/session/store";
import { api } from "#/lib/api";
import { asViewer, showPlanError } from "../actions";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));

const signedIn = (sessionId: string) =>
  useSessionStore.setState({
    state: { status: "signedIn", sessionId, me: { user: { id: sessionId } } } as never,
  });

describe("asViewer", () => {
  beforeEach(() => {
    jest.mocked(api).mockReturnValue({ service: () => ({}) } as never);
    signedIn("main");
  });

  it("hands back the answer while the account is still the one in use", async () => {
    await expect(asViewer(async () => "ok")).resolves.toBe("ok");
  });

  it("drops the answer once another account took over", async () => {
    const answer = asViewer(async () => {
      signedIn("ura");
      return "ok";
    });
    await expect(answer).resolves.toBeNull();
  });

  it("drops a refusal once another account took over, so the next account is not told of it", async () => {
    const alert = jest.spyOn(Alert, "alert").mockImplementation(() => undefined);
    const answer = asViewer(async () => {
      signedIn("ura");
      throw new Error("この予定は見つかりませんでした");
    }).catch((error: unknown) => {
      showPlanError(error);
      return "refused";
    });
    await expect(answer).resolves.toBeNull();
    expect(alert).not.toHaveBeenCalled();
  });
});
