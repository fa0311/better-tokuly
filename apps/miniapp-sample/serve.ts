/**
 * Serves the sample mini app (割り勘) for development and the browser tests:
 * its page, its script and the SDK, TypeScript with the types stripped, so
 * there is nothing to build. Not for production.
 *
 *   pnpm --filter @tokuly/miniapp-sample start        # http://127.0.0.1:5180
 *
 * Register `http://127.0.0.1:5180/` as a mini app's launch URL on a server
 * running with `TOKULY_DEV_MINI_APP_ALLOW_LOOPBACK=true`. The page accepts as
 * its host the Tokuly origins in `TOKULY_MINIAPP_HOSTS` (comma-separated;
 * the local dev servers by default).
 */
import { readFile } from "node:fs/promises";
import { createServer, type Server } from "node:http";
import { stripTypeScriptTypes } from "node:module";
import type { AddressInfo } from "node:net";
import { join, normalize } from "node:path";

const HERE = import.meta.dirname;
const SDK = join(HERE, "../../packages/miniapp/src");

export interface SampleServer {
  /** `http://127.0.0.1:<port>`. */
  origin: string;
  close(): Promise<void>;
}

/**
 * Starts the server. `pages` adds HTML pages by path (the browser tests'
 * hostile frame); `hostOrigins` are the Tokuly origins the page accepts.
 */
export async function serveSample({
  port = 0,
  hostOrigins,
  pages = {},
}: {
  port?: number;
  hostOrigins: string[];
  pages?: Record<string, string>;
}): Promise<SampleServer> {
  const server: Server = createServer((request, response) => {
    const path = new URL(request.url ?? "/", "http://sample").pathname;
    void respond(path, hostOrigins, pages).then(
      ({ status, type, body }) => {
        response.writeHead(status, { "content-type": type, "cache-control": "no-store" });
        response.end(body);
      },
      () => {
        response.writeHead(500).end();
      },
    );
  });
  await new Promise<void>((resolve) => server.listen(port, "127.0.0.1", resolve));
  const { port: bound } = server.address() as AddressInfo;
  return {
    origin: `http://127.0.0.1:${bound}`,
    close: () => new Promise((resolve) => server.close(() => resolve())),
  };
}

async function respond(
  path: string,
  hostOrigins: string[],
  pages: Record<string, string>,
): Promise<{ status: number; type: string; body: string }> {
  const page = pages[path];
  if (page !== undefined) return { status: 200, type: "text/html; charset=utf-8", body: page };
  if (path === "/") {
    const html = await readFile(join(HERE, "src/index.html"), "utf8");
    const hosts = hostOrigins.join(",").replace(/[<>"&]/g, "");
    return {
      status: 200,
      type: "text/html; charset=utf-8",
      body: html.replace("%HOST_ORIGINS%", hosts),
    };
  }
  // `/app.ts` and the SDK under `/sdk/`, nothing else.
  const file = path === "/app.ts" ? join(HERE, "src/app.ts") : sdkFile(path);
  if (!file) return { status: 404, type: "text/plain", body: "not found" };
  const source = await readFile(file, "utf8");
  return {
    status: 200,
    type: "text/javascript; charset=utf-8",
    body: stripTypeScriptTypes(source),
  };
}

function sdkFile(path: string): string | undefined {
  if (!path.startsWith("/sdk/") || !path.endsWith(".ts")) return undefined;
  const file = normalize(join(SDK, path.slice("/sdk/".length)));
  return file.startsWith(SDK) && !file.endsWith(".test.ts") ? file : undefined;
}

if (import.meta.main) {
  const hostOrigins = (
    process.env.TOKULY_MINIAPP_HOSTS ?? "http://localhost:3000,http://localhost:53000"
  ).split(",");
  const { origin } = await serveSample({ port: 5180, hostOrigins });
  console.log(`割り勘 is at ${origin}/ (hosts: ${hostOrigins.join(" ")})`);
}
