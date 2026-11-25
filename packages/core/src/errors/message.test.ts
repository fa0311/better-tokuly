import { Code, ConnectError } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { testI18n } from "../testing/i18n";
import { AssetFailedError, durationLimit, errorMessage } from "./index";

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

describe("errorMessage", () => {
  it("prefers the wording of the feature that shows the error", () => {
    expect(errorMessage(i18n, failure("ASSET_NOT_READY"), { namespace: "alpha" })).toBe(
      "alpha の準備中",
    );
    expect(errorMessage(i18n, failure("ASSET_NOT_READY"), { namespace: "chat" })).toBe(
      i18n.t("errors.ASSET_NOT_READY", { ns: "chat" }),
    );
  });

  it("falls back to the shared texts, then to any feature's in namespace order", () => {
    expect(errorMessage(i18n, failure("HANDLE_TAKEN"), { namespace: "chat" })).toBe(
      i18n.t("HANDLE_TAKEN", { ns: "errors" }),
    );
    expect(errorMessage(i18n, failure("POV_ALREADY_OPEN"))).toBe(
      i18n.t("errors.POV_ALREADY_OPEN", { ns: "chat" }),
    );
    expect(errorMessage(i18n, failure("SHARED_ONLY_HERE"))).toBe("alpha の文");
  });

  it("says something useful without a known reason", () => {
    expect(errorMessage(i18n, failure("SOMETHING_NEW"))).toBe(i18n.t("generic", { ns: "errors" }));
    expect(errorMessage(i18n, new TypeError("fetch failed"))).toBe(
      i18n.t("network", { ns: "errors" }),
    );
    expect(errorMessage(i18n, failure("", Code.ResourceExhausted))).toBe(
      i18n.t("RATE_LIMITED", { ns: "errors" }),
    );
  });

  it("says how long a refused recording may be", () => {
    expect(errorMessage(i18n, new AssetFailedError("MEDIA_TOO_LONG", 600))).toBe(
      "長すぎるよ!10分までにしてもう一度ためしてね",
    );
    expect(errorMessage(i18n, new AssetFailedError("MEDIA_TOO_LONG"))).toBe(
      i18n.t("MEDIA_TOO_LONG", { ns: "errors" }),
    );
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
