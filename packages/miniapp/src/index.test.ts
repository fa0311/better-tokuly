import { describe, expect, it } from "vitest";
import { BridgeError, connect, type Environment } from "./index.ts";
import { answer, type Request, readRequest, refuse } from "./protocol.ts";

/** A host that answers every request with `reply`. */
function host(reply: (request: Request) => unknown): Environment & { sent: Request[] } {
  let receive: (data: unknown) => void = () => undefined;
  const sent: Request[] = [];
  return {
    sent,
    send: (request) => {
      sent.push(request);
      const read = readRequest(request);
      expect(read && "call" in read).toBe(true);
      queueMicrotask(() => receive(reply(request)));
    },
    listen: (next) => {
      receive = next;
    },
  };
}

const launch = { path: "/", language: "ja", inConversation: true, grantedScopes: [] };

describe("connect", () => {
  it("says hello, then makes calls and reads their answers", async () => {
    const environment = host((request) => {
      if (request.call.method === "hello") return answer(request.id, launch);
      if (request.call.method === "getProfile")
        return answer(request.id, { id: "u1", name: "ゆい", handle: "yui", avatarUrl: "" });
      return refuse(request.id, "CANCELLED", "closed");
    });
    const tokuly = await connect({ hostOrigins: [] }, environment);
    expect(tokuly.launch).toEqual(launch);
    expect((await tokuly.getProfile()).name).toBe("ゆい");
    const refused = await tokuly.pickFriends({ max: 3 }).catch((error: unknown) => error);
    expect(refused).toBeInstanceOf(BridgeError);
    expect((refused as BridgeError).code).toBe("CANCELLED");
    expect(environment.sent.map((request) => request.call.method)).toEqual([
      "hello",
      "getProfile",
      "pickFriends",
    ]);
  });

  it("ignores answers it did not ask for", async () => {
    const environment = host((request) =>
      request.call.method === "hello"
        ? answer(request.id, launch)
        : answer("someone-else", { id: "x" }),
    );
    const tokuly = await connect({ hostOrigins: [], timeoutMs: 20 }, environment);
    const card = tokuly.shareCard({ title: "割り勘" });
    let settled = false;
    void card.finally(() => {
      settled = true;
    });
    await new Promise((resolve) => setTimeout(resolve, 50));
    // Waiting for the person is never timed out.
    expect(settled).toBe(false);
  });

  it("gives up on a host that never answers hello", async () => {
    const silent: Environment = { send: () => undefined, listen: () => undefined };
    await expect(connect({ hostOrigins: [], timeoutMs: 10 }, silent)).rejects.toMatchObject({
      code: "TIMEOUT",
    });
  });
});
