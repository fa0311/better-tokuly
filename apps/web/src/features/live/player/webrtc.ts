/**
 * WebRTC against the media server, by the two HTTP handshakes it speaks:
 * WHEP to watch (sub-second latency) and WHIP to publish from the browser.
 * Both are one POST of an SDP offer answered with the server's SDP.
 *
 * The URLs and tokens handed in here are secrets (a playback token, a stream
 * key): they go into the request and nowhere else. Errors carry the HTTP
 * status only, never the address.
 */

/** A media session on the server; `close` ends it there and here. */
export interface MediaSession {
  connection: RTCPeerConnection;
  close(): void;
}

export class MediaSessionError extends Error {
  constructor(
    /** HTTP status of the handshake; 0 when the request never got an answer. */
    readonly status: number,
  ) {
    super(`media handshake failed (${status})`);
  }
}

/** How long to collect ICE candidates before sending the offer as it stands. */
const GATHER_TIMEOUT_MS = 1500;

function gathered(connection: RTCPeerConnection): Promise<void> {
  if (connection.iceGatheringState === "complete") return Promise.resolve();
  return new Promise((resolve) => {
    const done = () => {
      clearTimeout(timer);
      connection.removeEventListener("icegatheringstatechange", check);
      resolve();
    };
    const check = () => connection.iceGatheringState === "complete" && done();
    const timer = setTimeout(done, GATHER_TIMEOUT_MS);
    connection.addEventListener("icegatheringstatechange", check);
  });
}

async function handshake(
  connection: RTCPeerConnection,
  url: string,
  bearer: string,
  signal: AbortSignal,
): Promise<MediaSession> {
  await connection.setLocalDescription(await connection.createOffer());
  await gathered(connection);
  let response: Response;
  try {
    response = await fetch(url, {
      method: "POST",
      headers: {
        "content-type": "application/sdp",
        ...(bearer ? { authorization: `Bearer ${bearer}` } : {}),
      },
      body: connection.localDescription?.sdp ?? "",
      signal,
      // The address may carry a token: it must not travel on as a referrer.
      referrerPolicy: "no-referrer",
    });
  } catch {
    throw new MediaSessionError(0);
  }
  if (!response.ok) throw new MediaSessionError(response.status);
  const location = response.headers.get("location");
  const sessionUrl = location ? new URL(location, url).href : "";
  await connection.setRemoteDescription({ type: "answer", sdp: await response.text() });
  let closed = false;
  return {
    connection,
    close() {
      if (closed) return;
      closed = true;
      connection.close();
      // Tell the server at once instead of letting the session time out.
      if (sessionUrl)
        void fetch(sessionUrl, {
          method: "DELETE",
          headers: bearer ? { authorization: `Bearer ${bearer}` } : {},
          referrerPolicy: "no-referrer",
          keepalive: true,
        }).catch(() => {});
    },
  };
}

/**
 * Starts watching over WHEP. The returned session's tracks arrive on
 * `onTrack` as the connection comes up.
 */
export async function watchOverWhep(
  whepUrl: string,
  token: string,
  onTrack: (stream: MediaStream) => void,
  signal: AbortSignal,
): Promise<MediaSession> {
  const connection = new RTCPeerConnection();
  const media = new MediaStream();
  connection.addTransceiver("video", { direction: "recvonly" });
  connection.addTransceiver("audio", { direction: "recvonly" });
  connection.addEventListener("track", (event) => {
    media.addTrack(event.track);
    onTrack(media);
  });
  try {
    // The token is already part of the address the API hands out; the header covers servers that want it there.
    return await handshake(connection, whepUrl, whepUrl.includes("?") ? "" : token, signal);
  } catch (error) {
    connection.close();
    throw error;
  }
}

/** Puts H.264 first: the media server can then serve HLS, rewinding and the archive from what is sent. */
function preferH264(transceiver: RTCRtpTransceiver): void {
  const codecs = RTCRtpReceiver.getCapabilities("video")?.codecs;
  if (!codecs || typeof transceiver.setCodecPreferences !== "function") return;
  const h264 = codecs.filter((codec) => codec.mimeType.toLowerCase() === "video/h264");
  if (h264.length === 0) return;
  const rest = codecs.filter((codec) => codec.mimeType.toLowerCase() !== "video/h264");
  try {
    transceiver.setCodecPreferences([...h264, ...rest]);
  } catch {
    // The browser's own order still works for WebRTC viewers.
  }
}

/** Publishes a camera or screen capture over WHIP with the stream key as bearer token. */
export async function publishOverWhip(
  whipUrl: string,
  bearer: string,
  media: MediaStream,
  signal: AbortSignal,
): Promise<MediaSession> {
  const connection = new RTCPeerConnection();
  for (const track of media.getTracks()) {
    const transceiver = connection.addTransceiver(track, {
      direction: "sendonly",
      streams: [media],
    });
    if (track.kind === "video") preferH264(transceiver);
  }
  try {
    return await handshake(connection, whipUrl, bearer, signal);
  } catch (error) {
    connection.close();
    throw error;
  }
}
