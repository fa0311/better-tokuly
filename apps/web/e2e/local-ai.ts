import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import http from "node:http";
import type { AddressInfo } from "node:net";
import os from "node:os";
import path from "node:path";
import type { APIRequestContext } from "@playwright/test";
import { type Account, rpc } from "./support";

// The AI provider is one setting for the whole server, and the browser tests
// run in parallel: a test that points it at its local stand-in holds it
// (`holdAiProvider`) until it puts it back, so two never overwrite each other.

/**
 * Where the API reaches this machine: loopback when it runs here, the Compose
 * network's gateway when it runs in a container (set by the End to end
 * workflow). A stand-in listens on that address only.
 */
export const STUB_HOST = process.env.TOKULY_E2E_HOST_FROM_API ?? "127.0.0.1";

const ASSIST_ADMIN = "tokuly.chat.v1.AssistAdminService";
/** A holder that crashed leaves the lock behind; past this age it is taken over. */
const STALE_MS = 5 * 60_000;

/**
 * Waits until no other test of this run's server holds the AI provider, then
 * holds it. Resolves to the release.
 */
export async function holdAiProvider(): Promise<() => Promise<void>> {
  const server = process.env.TOKULY_WEB_URL ?? "http://localhost:3000";
  const name = createHash("sha256").update(server).digest("hex").slice(0, 12);
  const lock = path.join(os.tmpdir(), `tokuly-e2e-ai-${name}`);
  for (;;) {
    try {
      await fs.mkdir(lock);
      return () => fs.rm(lock, { recursive: true, force: true });
    } catch {
      const held = await fs.stat(lock).catch(() => undefined);
      if (held && Date.now() - held.mtimeMs > STALE_MS)
        await fs.rm(lock, { recursive: true, force: true });
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }
}

export interface Stub {
  baseUrl: string;
  requests: { body: Record<string, unknown> }[];
  close(): Promise<void>;
}

/**
 * A local server speaking OpenAI's Chat Completions protocol, like LM Studio
 * on the operator's network. `answer` gets the system prompt and the user
 * turn and returns the JSON object to answer with; `schemas: false` refuses
 * `response_format` like a server without schema support.
 */
export async function startStub(
  answer: (system: string, prompt: string) => Record<string, unknown>,
  { schemas = true }: { schemas?: boolean } = {},
): Promise<Stub> {
  const requests: Stub["requests"] = [];
  const server = http.createServer((incoming, outgoing) => {
    let raw = "";
    incoming.on("data", (chunk) => {
      raw += chunk;
    });
    incoming.on("end", () => {
      const body = JSON.parse(raw || "{}") as Record<string, unknown>;
      requests.push({ body });
      outgoing.setHeader("content-type", "application/json");
      if (body.response_format && !schemas) {
        outgoing.statusCode = 400;
        outgoing.end(JSON.stringify({ error: "response_format is not supported" }));
        return;
      }
      const messages = body.messages as { content: string }[];
      const content = JSON.stringify(
        answer(messages[0]?.content ?? "", messages[1]?.content ?? ""),
      );
      outgoing.end(
        JSON.stringify({
          choices: [
            {
              index: 0,
              message: {
                role: "assistant",
                content: schemas ? content : `\`\`\`json\n${content}\n\`\`\``,
              },
              finish_reason: "stop",
            },
          ],
        }),
      );
    });
  });
  await new Promise<void>((resolve) => server.listen(0, STUB_HOST, resolve));
  const { port } = server.address() as AddressInfo;
  return {
    baseUrl: `http://${STUB_HOST}:${port}/v1`,
    requests,
    close: () => new Promise((resolve) => server.close(() => resolve())),
  };
}

/** Points the OpenAI-compatible provider at `stub` (through the private-host allowance). */
export async function offerStub(
  request: APIRequestContext,
  admin: Account,
  stub: Stub,
  model = "local-qwen",
): Promise<void> {
  await rpc(
    request,
    `${ASSIST_ADMIN}/UpdateAssistProvider`,
    {
      provider: "AI_PROVIDER_OPENAI_COMPATIBLE",
      enabled: true,
      model,
      baseUrl: stub.baseUrl,
      allowPrivateHost: true,
    },
    admin,
  );
}

/** Leaves the server as it was: no compatible provider. */
export async function withdrawStub(request: APIRequestContext, admin: Account): Promise<void> {
  await rpc(
    request,
    `${ASSIST_ADMIN}/UpdateAssistProvider`,
    { provider: "AI_PROVIDER_OPENAI_COMPATIBLE", enabled: false, model: "", baseUrl: "" },
    admin,
  ).catch(() => undefined);
}
