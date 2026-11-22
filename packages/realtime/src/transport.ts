import { encodeFrame, FrameDecoder } from "./framing.js";

/**
 * A connected realtime transport. Both implementations carry opaque frame
 * bytes; the client above them handles the protocol.
 */
export interface FrameTransport {
  readonly kind: "webtransport" | "websocket";
  /** Sends a frame reliably. */
  send(frame: Uint8Array): void;
  /**
   * Sends a frame that may be dropped (location samples). Falls back to the
   * reliable path when the transport has no unreliable channel.
   */
  sendUnreliable(frame: Uint8Array): void;
  close(): void;
}

export interface TransportHandlers {
  onFrame(frame: Uint8Array): void;
  /** Called once when the transport is gone, for whatever reason. */
  onClose(reason: string): void;
}

export interface Endpoints {
  webtransportUrl: string;
  /** SHA-256 certificate hashes to pin when the gateway's certificate is self-signed. */
  webtransportCertHashes: Uint8Array[];
  websocketUrl: string;
}

/**
 * Frames are plain (non-shared) byte arrays; the DOM send APIs only accept
 * that flavour, which the typed-array generics cannot infer on their own.
 */
function sendable(frame: Uint8Array): Uint8Array<ArrayBuffer> {
  return frame as Uint8Array<ArrayBuffer>;
}

/** Datagrams larger than this are sent on the reliable stream instead. */
const MAX_DATAGRAM_BYTES = 1100;

export function webTransportSupported(): boolean {
  return typeof (globalThis as { WebTransport?: unknown }).WebTransport === "function";
}

export async function connectWebTransport(
  endpoints: Endpoints,
  handlers: TransportHandlers,
  timeoutMs: number,
): Promise<FrameTransport> {
  const options: WebTransportOptions = {};
  if (endpoints.webtransportCertHashes.length > 0) {
    options.serverCertificateHashes = endpoints.webtransportCertHashes.map((value) => ({
      algorithm: "sha-256",
      value: sendable(value),
    }));
  }
  const session = new WebTransport(endpoints.webtransportUrl, options);
  let closed = false;
  const closeOnce = (reason: string) => {
    if (closed) return;
    closed = true;
    try {
      session.close();
    } catch {
      // already closed
    }
    handlers.onClose(reason);
  };

  await withTimeout(session.ready, timeoutMs, () => session.close());
  const stream = await withTimeout(session.createBidirectionalStream(), timeoutMs, () =>
    session.close(),
  );
  const writer = stream.writable.getWriter();
  const datagramWriter = session.datagrams.writable.getWriter();

  void session.closed.then(
    () => closeOnce("closed"),
    (error) => closeOnce(String(error)),
  );

  // Reliable frames.
  void (async () => {
    const decoder = new FrameDecoder();
    const reader = stream.readable.getReader();
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        for (const frame of decoder.push(value)) handlers.onFrame(frame);
      }
      closeOnce("stream ended");
    } catch (error) {
      closeOnce(String(error));
    }
  })();

  // Datagrams: one frame each.
  void (async () => {
    const reader = session.datagrams.readable.getReader();
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        handlers.onFrame(value);
      }
    } catch {
      // The session close handler reports the failure.
    }
  })();

  const maxDatagram = Math.min(
    MAX_DATAGRAM_BYTES,
    session.datagrams.maxDatagramSize || MAX_DATAGRAM_BYTES,
  );
  const send = (frame: Uint8Array) => {
    writer.write(sendable(encodeFrame(frame))).catch((error) => closeOnce(String(error)));
  };
  return {
    kind: "webtransport",
    send,
    sendUnreliable(frame) {
      if (frame.length <= maxDatagram) {
        // A full datagram queue just drops the sample, which is the point.
        datagramWriter.write(sendable(frame)).catch(() => {});
      } else {
        send(frame);
      }
    },
    close: () => closeOnce("client closed"),
  };
}

export function connectWebSocket(
  endpoints: Endpoints,
  handlers: TransportHandlers,
  timeoutMs: number,
): Promise<FrameTransport> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(endpoints.websocketUrl);
    socket.binaryType = "arraybuffer";
    let opened = false;
    let closed = false;
    const timer = setTimeout(() => {
      if (!opened) socket.close();
    }, timeoutMs);
    const closeOnce = (reason: string) => {
      if (closed) return;
      closed = true;
      clearTimeout(timer);
      if (opened) handlers.onClose(reason);
      else reject(new Error(`websocket failed: ${reason}`));
    };
    const transport: FrameTransport = {
      kind: "websocket",
      send(frame) {
        if (socket.readyState === WebSocket.OPEN) socket.send(sendable(frame));
      },
      sendUnreliable(frame) {
        // Skip droppable frames while the socket is backed up.
        if (socket.readyState === WebSocket.OPEN && socket.bufferedAmount < 64 * 1024) {
          socket.send(sendable(frame));
        }
      },
      close() {
        socket.close();
        closeOnce("client closed");
      },
    };
    socket.onopen = () => {
      opened = true;
      clearTimeout(timer);
      resolve(transport);
    };
    socket.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) handlers.onFrame(new Uint8Array(event.data));
    };
    socket.onerror = () => closeOnce("error");
    socket.onclose = (event) => closeOnce(event.reason || `closed (${event.code})`);
  });
}

function withTimeout<T>(promise: Promise<T>, timeoutMs: number, onTimeout: () => void): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      onTimeout();
      reject(new Error("timed out"));
    }, timeoutMs);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}
