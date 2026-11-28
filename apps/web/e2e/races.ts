import { expect, type Page, type WebSocketRoute } from "@playwright/test";

/**
 * The race of `docs/realtime-consistency.md`, rule 2, set up in a browser: a
 * read whose answer the server made before a push, landing after the push.
 *
 * `holdAnswers` lets the page's requests to `procedure` (as `rpc` takes it) reach
 * the server at once and keeps their answers from the page until `release()`.
 * `pushes` counts the realtime frames that reach the page and name `needle`
 * (an ID: frames are protobuf, its text is in them as it is). Both keep the
 * page on WebSocket, which Playwright can watch; call them before the page
 * loads.
 */
export async function holdAnswers(page: Page, procedure: string) {
  await onWebSocket(page);
  let holding = true;
  let asked = 0;
  let answered = 0;
  const held: (() => Promise<void>)[] = [];
  await page.route(
    (url) => url.pathname.endsWith(`/${procedure}`),
    async (route) => {
      if (!holding) return route.fallback();
      asked++;
      const response = await route.fetch();
      answered++;
      if (holding) held.push(() => route.fulfill({ response }));
      else await route.fulfill({ response });
    },
  );
  return {
    /** Whether the page asked, and every request it made so far has its answer from the server. */
    answered: () => asked > 0 && answered === asked,
    async release() {
      holding = false;
      for (const fulfill of held.splice(0)) await fulfill();
    },
  };
}

/** Counts the realtime frames the page receives that contain `needle`; any frame when it is empty. */
export async function pushes(page: Page, needle = "") {
  await onWebSocket(page);
  const bytes = Buffer.from(needle);
  let count = 0;
  page.on("websocket", (socket) => {
    if (!socket.url().includes("/rt/")) return;
    socket.on("framereceived", ({ payload }) => {
      const frame = typeof payload === "string" ? Buffer.from(payload) : payload;
      if (frame.includes(bytes)) count++;
    });
  });
  return () => count;
}

/**
 * Lets the test drop the page's realtime connection, on WebSocket, which Playwright can stand
 * in. `drop()` closes the connection in use once it was welcomed (the gateway's first frame);
 * what it returns says whether the connection after it was welcomed yet: what the page reads
 * from then on (rule 1 after a reconnect) is made after the subscriptions are back. Call before
 * the page loads.
 */
export async function droppableConnection(page: Page) {
  await onWebSocket(page);
  const sockets: { ws: WebSocketRoute; welcomed: boolean }[] = [];
  await page.routeWebSocket(/\/rt\/ws/, (ws) => {
    const socket = { ws, welcomed: false };
    sockets.push(socket);
    const server = ws.connectToServer();
    ws.onMessage((message) => server.send(message));
    server.onMessage((message) => {
      socket.welcomed = true;
      ws.send(message);
    });
  });
  return {
    async drop() {
      await expect.poll(() => sockets.at(-1)?.welcomed).toBe(true);
      const next = sockets.length;
      await sockets.at(-1)?.ws.close();
      return () => sockets[next]?.welcomed === true;
    },
  };
}

const onWebSocketPages = new WeakSet<Page>();

async function onWebSocket(page: Page) {
  if (onWebSocketPages.has(page)) return;
  onWebSocketPages.add(page);
  await page.addInitScript(() => {
    Reflect.deleteProperty(globalThis, "WebTransport");
  });
}
