import { Code, ConnectError } from "@tokuly/api";
import { ja } from "@tokuly/i18n";
import { createInstance } from "i18next";
import { afterEach, beforeEach, describe, expect, it, type MockInstance, vi } from "vitest";
import { testI18n } from "../testing/i18n";
import {
  AssetFailedError,
  durationLimit,
  errorMessage,
  LocalFileError,
  NetworkError,
  reasonText,
  TransferError,
} from "./index";

/** An API error carrying a reason, as the server sends it. */
const failure = (reason: string, code = Code.FailedPrecondition) =>
  new ConnectError("failed", code, new Headers({ "x-tokuly-error": reason }));

// Two features of an app's own with texts for the same reasons, to see the order rules.
const i18n = testI18n("ja", {
  ja: {
    alpha: { errors: { SHARED_ONLY_HERE: "alpha の文", ASSET_NOT_READY: "alpha の準備中" } },
    zulu: { errors: { SHARED_ONLY_HERE: "zulu の文" } },
  },
});

// What falls back to the generic text is logged: kept out of the test output, counted where it matters.
let logs: MockInstance<typeof console.error>;
beforeEach(() => {
  logs = vi.spyOn(console, "error").mockImplementation(() => {});
});
afterEach(() => {
  vi.restoreAllMocks();
});

describe("errorMessage", () => {
  it("prefers the wording of the feature that shows the error", () => {
    expect(errorMessage(i18n, failure("ASSET_NOT_READY"), { namespace: "alpha" })).toBe(
      "alpha の準備中",
    );
    expect(errorMessage(i18n, failure("ASSET_NOT_READY"), { namespace: "stamps" })).toBe(
      i18n.t("errors.ASSET_NOT_READY", { ns: "stamps" }),
    );
  });

  it("falls back to the shared text, which has every reason", () => {
    expect(errorMessage(i18n, failure("HANDLE_TAKEN"), { namespace: "chat" })).toBe(
      i18n.t("HANDLE_TAKEN", { ns: "errors" }),
    );
    expect(errorMessage(i18n, failure("SHARED_ONLY_HERE"), { namespace: "alpha" })).toBe(
      "alpha の文",
    );
    // Another feature's wording is never borrowed: what a screen says does not depend on what else is loaded.
    expect(errorMessage(i18n, failure("SHARED_ONLY_HERE"))).toBe(
      i18n.t("generic", { ns: "errors" }),
    );
  });

  it("needs no feature's strings loaded", () => {
    // The web loads a feature's strings when one of its screens opens; `errors` is always there.
    const onlyErrors = createInstance();
    void onlyErrors.init({
      resources: { ja: { errors: ja.errors } },
      lng: "ja",
      initAsync: false,
    });
    expect(errorMessage(onlyErrors, failure("POV_ALREADY_OPEN"))).toBe(ja.errors.POV_ALREADY_OPEN);
    expect(errorMessage(onlyErrors, failure("TOO_MANY_PINNED"), { namespace: "notes" })).toBe(
      ja.errors.TOO_MANY_PINNED,
    );
  });

  it("says something useful without a known reason", () => {
    expect(errorMessage(i18n, failure("SOMETHING_NEW"))).toBe(i18n.t("generic", { ns: "errors" }));
    expect(errorMessage(i18n, failure("", Code.ResourceExhausted))).toBe(
      i18n.t("RATE_LIMITED", { ns: "errors" }),
    );
    expect(logs).toHaveBeenCalledTimes(1);
  });

  it("blames the connection only when the request got no answer", () => {
    const network = i18n.t("network", { ns: "errors" });
    for (const message of [
      "fetch failed",
      "Failed to fetch",
      "Load failed",
      "Network request failed",
    ])
      expect(errorMessage(i18n, new TypeError(message)), message).toBe(network);
    // A fetch that failed, as Connect throws it.
    const lost = new ConnectError(
      "fetch failed",
      Code.Unknown,
      undefined,
      undefined,
      new TypeError("Failed to fetch"),
    );
    expect(errorMessage(i18n, lost)).toBe(network);
    expect(errorMessage(i18n, new ConnectError("down", Code.Unavailable))).toBe(network);
    expect(errorMessage(i18n, new ConnectError("slow", Code.DeadlineExceeded))).toBe(network);
    expect(errorMessage(i18n, new NetworkError())).toBe(network);
  });

  it("says what went wrong with an upload or a file on the device", () => {
    expect(errorMessage(i18n, new TransferError(413))).toBe(
      i18n.t("uploadTooLarge", { ns: "errors" }),
    );
    expect(errorMessage(i18n, new TransferError(500))).toBe(
      i18n.t("uploadRefused", { ns: "errors" }),
    );
    expect(errorMessage(i18n, new LocalFileError())).toBe(
      i18n.t("fileUnreadable", { ns: "errors" }),
    );
  });

  it("calls a bug a bug, and logs it once", () => {
    const generic = i18n.t("generic", { ns: "errors" });
    const bug = new TypeError("cannot read properties of undefined");
    expect(errorMessage(i18n, bug)).toBe(generic);
    expect(errorMessage(i18n, bug)).toBe(generic);
    expect(errorMessage(i18n, new RangeError("invalid time zone"))).toBe(generic);
    expect(errorMessage(i18n, new ConnectError("panic", Code.Unknown))).toBe(generic);
    expect(errorMessage(i18n, new ConnectError("oops", Code.Internal))).toBe(generic);
    expect(logs).toHaveBeenCalledTimes(4);
  });

  it("says how long a refused recording may be", () => {
    expect(errorMessage(i18n, new AssetFailedError("MEDIA_TOO_LONG", 600))).toBe(
      "長すぎるよ!10分までにしてもう一度ためしてね",
    );
    expect(errorMessage(i18n, new AssetFailedError("MEDIA_TOO_LONG"))).toBe(
      i18n.t("MEDIA_TOO_LONG", { ns: "errors" }),
    );
  });

  it("says a file was refused when the reason has no text", () => {
    expect(errorMessage(i18n, new AssetFailedError("SOMETHING_NEW"))).toBe(
      i18n.t("ASSET_FAILED", { ns: "errors" }),
    );
    expect(errorMessage(i18n, new AssetFailedError("SOMETHING_NEW"), { namespace: "stamps" })).toBe(
      i18n.t("errors.ASSET_FAILED", { ns: "stamps" }),
    );
  });
});

describe("reasonText", () => {
  it("words a reason that came without an error, or says it does not know it", () => {
    expect(reasonText(i18n, "ASSET_NOT_READY", { namespace: "alpha" })).toBe("alpha の準備中");
    expect(reasonText(i18n, "HANDLE_TAKEN")).toBe(i18n.t("HANDLE_TAKEN", { ns: "errors" }));
    expect(reasonText(i18n, "SOMETHING_NEW")).toBeUndefined();
  });
});

describe("durationLimit", () => {
  it("counts seconds up to two minutes, then minutes or hours", () => {
    expect(durationLimit(i18n, 15)).toBe("15秒");
    expect(durationLimit(i18n, 120)).toBe("120秒");
    expect(durationLimit(i18n, 180)).toBe("3分");
    expect(durationLimit(i18n, 3 * 3600)).toBe("3時間");
    expect(durationLimit(testI18n("en"), 180)).toBe(
      testI18n("en").t("limit.minutes", { ns: "errors", count: 3 }),
    );
  });
});
