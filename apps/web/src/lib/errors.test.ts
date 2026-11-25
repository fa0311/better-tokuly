import { Code, ConnectError } from "@tokuly/api";
import { AssetFailedError } from "@tokuly/core/errors";
import { describe, expect, it } from "vitest";
import { durationLimit, errorMessage } from "./errors";

/** An API error carrying a reason, as the server sends it. */
const failure = (reason: string, code = Code.FailedPrecondition) =>
  new ConnectError("failed", code, new Headers({ "x-tokuly-error": reason }));

describe("error messages", () => {
  it("prefer the wording of the feature that shows the error", () => {
    // Both chat and stamps can hit an upload that is not ready; each says it its own way.
    expect(errorMessage(failure("ASSET_NOT_READY"), { namespace: "stamps" })).toBe(
      "画像の準備がまだ終わっていません",
    );
    expect(errorMessage(failure("ASSET_NOT_READY"), { namespace: "chat" })).toBe(
      "ファイルの準備がまだ終わっていません。少し待ってね",
    );
    expect(errorMessage(failure("FILE_TOO_LARGE"), { namespace: "stamps" })).toBe(
      "ファイルが大きすぎます",
    );
  });

  it("fall back to the shared texts when the feature has none of its own", () => {
    expect(errorMessage(failure("HANDLE_TAKEN"), { namespace: "chat" })).toBe(
      "このハンドルはもう使われています",
    );
    expect(errorMessage(failure("HANDLE_TAKEN"))).toBe("このハンドルはもう使われています");
  });

  it("then use another feature's text rather than a generic apology", () => {
    // Only chat knows this reason; a caller from elsewhere (or one that names no feature) still gets it.
    expect(errorMessage(failure("POV_ALREADY_OPEN"), { namespace: "stamps" })).toBe(
      "まだ答えを待っているPOVリクエストがあるよ",
    );
    expect(errorMessage(failure("POV_ALREADY_OPEN"))).toBe(
      "まだ答えを待っているPOVリクエストがあるよ",
    );
  });

  it("leave callers that name no feature with the wording they had", () => {
    // A bad cursor: three features have a text; without a namespace the first one in order still wins, as before.
    expect(errorMessage(failure("INVALID_CURSOR"))).toBe(
      "一覧の続きを読み込めませんでした。ページを開き直してね",
    );
    expect(errorMessage(failure("INVALID_CURSOR"), { namespace: "wallet" })).toBe(
      "一覧を読み込み直してね",
    );
    expect(errorMessage(failure("INVALID_CURSOR"), { namespace: "chat" })).toBe(
      "つづきを読み込めませんでした。ひらき直してみてね",
    );
  });

  it("say something useful without a known reason", () => {
    expect(errorMessage(failure("SOMETHING_NEW"))).toBe(
      "うまくいきませんでした。もう一度ためしてね",
    );
    expect(errorMessage(new TypeError("fetch failed"))).toBe(
      "通信できませんでした。接続を確認してね",
    );
    expect(errorMessage(failure("", Code.ResourceExhausted))).toBe(
      "ちょっと待ってから、もう一度ためしてね",
    );
  });

  it("say how long a refused recording may be", () => {
    expect(errorMessage(new AssetFailedError("MEDIA_TOO_LONG", 600))).toBe(
      "長すぎるよ!10分までにしてもう一度ためしてね",
    );
    expect(durationLimit(60)).toBe("60秒");
    expect(durationLimit(15)).toBe("15秒");
    expect(durationLimit(180)).toBe("3分");
    expect(durationLimit(3 * 3600)).toBe("3時間");
    // Without a known limit, the plain reason.
    expect(errorMessage(new AssetFailedError("MEDIA_TOO_LONG"))).toBe(
      "長すぎます。もう少し短いものでためしてね",
    );
    expect(errorMessage(new AssetFailedError("DAILY_TRANSCODE_LIMIT_EXCEEDED"))).toBe(
      "今日変換できる動画の長さの上限に達したよ。また明日ためしてね!",
    );
  });
});
