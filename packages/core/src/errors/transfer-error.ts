/**
 * Failures that happen outside the API, on the way to object storage or on
 * the device, thrown as these so `errorMessage` can say what went wrong.
 */

/** A PUT to object storage was answered with something other than 2xx (413: the file is too large). */
export class TransferError extends Error {
  readonly status: number;

  constructor(status: number) {
    super(`upload failed (${status})`);
    this.name = "TransferError";
    this.status = status;
  }
}

/** A request that never got an answer: no connection, or it broke off on the way. */
export class NetworkError extends Error {
  constructor(message = "network request failed", options?: { cause?: unknown }) {
    super(message, options);
    this.name = "NetworkError";
  }
}

/** A file on the device that could not be read or copied (a picked photo the OS took back). */
export class LocalFileError extends Error {
  constructor(message = "the file cannot be read", options?: { cause?: unknown }) {
    super(message, options);
    this.name = "LocalFileError";
  }
}
