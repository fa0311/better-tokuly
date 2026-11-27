import { type Api, media } from "@tokuly/api";
import * as uploads from "@tokuly/core/media";
import { api } from "#/lib/api";
import { SERVER_URL } from "#/lib/config";

const mediaClient = (options: { api?: Api }) => () =>
  (options.api ?? api()).service(media.MediaService);

/** A file on the device (a photo from the library or the camera), as the pickers hand it over. */
export interface LocalFile {
  uri: string;
  name: string;
  type: string;
}

export interface UploadOptions {
  purpose: media.AssetPurpose;
  /** 0..1 as bytes reach the object store. */
  onProgress?(fraction: number): void;
  /** The account the upload belongs to (`Account.api`); the active one when left out. */
  api?: Api;
}

/** An upload URL the server gave as a path is on the app's own server. */
const absolute = (url: string) => (url.startsWith("/") ? `${SERVER_URL}${url}` : url);

/** XMLHttpRequest, because fetch cannot report upload progress. */
function put(
  url: string,
  body: Blob,
  headers: Record<string, string>,
  onProgress: (loaded: number) => void,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open("PUT", absolute(url));
    for (const [name, value] of Object.entries(headers)) request.setRequestHeader(name, value);
    request.upload.onprogress = (event) => onProgress(event.loaded);
    request.onload = () =>
      request.status >= 200 && request.status < 300
        ? resolve()
        : reject(new Error(`upload failed (${request.status})`));
    request.onerror = () => reject(new Error("upload failed (network)"));
    request.send(body);
  });
}

/**
 * The web's upload flow on the phone: the bytes go straight to object
 * storage (one request, or parts for a large file), then the asset is
 * processed and waited for. A half-finished upload is aborted.
 */
export async function uploadAndProcess(
  file: LocalFile,
  options: UploadOptions,
): Promise<media.Asset> {
  const blob = await (await fetch(file.uri)).blob();
  const client = mediaClient(options);
  const asset = await uploads.uploadBytes(
    client,
    {
      purpose: options.purpose,
      fileName: file.name,
      contentType: file.type || blob.type || "application/octet-stream",
    },
    blob,
    put,
    options,
  );
  return uploads.processed(client, asset);
}
