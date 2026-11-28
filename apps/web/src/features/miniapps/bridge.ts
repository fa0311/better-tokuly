import { type Call, type Response, readRequest, refuse } from "@tokuly/miniapp/protocol";

/**
 * The host's end of the bridge on the web: what the mini app in `frame` asks
 * for, and the way back to it.
 *
 * The frame is sandboxed without `allow-same-origin`, so its documents have
 * an opaque origin ("null") and cannot reach this page, its storage or its
 * cookies. A message counts only when it comes from that frame's window and
 * from an opaque origin; anything else on the page (another frame, an
 * extension, this window itself) is not the mini app and is not answered.
 */
export function listenToFrame(
  frame: HTMLIFrameElement,
  onCall: (id: string, call: Call) => void,
): () => void {
  const listener = (event: MessageEvent) => {
    if (event.source === null || event.source !== frame.contentWindow) return;
    if (event.origin !== "null") return;
    const read = readRequest(event.data);
    if (!read) return;
    if ("error" in read) reply(frame, refuse(read.id, read.error.code, read.error.message));
    else onCall(read.id, read.call);
  };
  window.addEventListener("message", listener);
  return () => window.removeEventListener("message", listener);
}

/**
 * Answers the mini app. An opaque origin cannot be named as the target, so
 * the answer goes to the frame's window under `*`: what that window shows is
 * held to the approved origin by the page's `frame-src`, so nobody else can
 * be in it to read the answer.
 */
export function reply(frame: HTMLIFrameElement, response: Response): void {
  frame.contentWindow?.postMessage(response, "*");
}
