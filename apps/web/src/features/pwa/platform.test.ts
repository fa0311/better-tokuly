import { describe, expect, it } from "vitest";
import { isIos, manualInstall } from "./platform";

const IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko)";
const UA = {
  iphoneSafari: `${IPHONE} Version/18.5 Mobile/15E148 Safari/604.1`,
  iphoneChrome: `${IPHONE} CriOS/138.0.7204.156 Mobile/15E148 Safari/604.1`,
  iphoneLine: `${IPHONE} Mobile/15E148 Safari Line/15.9.0`,
  iphoneInstagram: `${IPHONE} Mobile/22F76 Instagram 386.0.0.21.83 (iPhone16,1; iOS 18_5; ja_JP; ja; scale=3.00; 1179x2556; 745465538)`,
  iphoneWebView: `${IPHONE} Mobile/15E148`,
  ipadSafari: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15",
  macSafari: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15",
  macChrome: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36",
  androidChrome: "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Mobile Safari/537.36",
  androidLine: "Mozilla/5.0 (Linux; Android 15; Pixel 9; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/138.0.0.0 Mobile Safari/537.36 Line/15.9.0/IAB",
  androidFirefox: "Mozilla/5.0 (Android 15; Mobile; rv:140.0) Gecko/140.0 Firefox/140.0",
  windowsEdge: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36 Edg/138.0.0.0",
  windowsFirefox: "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0",
};

describe("isIos", () => {
  it("knows iPhones, and iPads that call themselves a Mac", () => {
    expect(isIos(UA.iphoneSafari, 5)).toBe(true);
    expect(isIos(UA.ipadSafari, 5)).toBe(true);
    expect(isIos(UA.macSafari, 0)).toBe(false);
    expect(isIos(UA.androidChrome, 5)).toBe(false);
  });
});

describe("manualInstall", () => {
  it("sends iPhone Safari to the share menu", () => {
    expect(manualInstall(UA.iphoneSafari, 5)).toBe("ios-share");
    expect(manualInstall(UA.ipadSafari, 5)).toBe("ios-share");
  });

  it("knows other browsers on iOS have the entry in their own share menu", () => {
    expect(manualInstall(UA.iphoneChrome, 5)).toBe("ios-browser");
  });

  it("asks for Safari when the page is open inside another app", () => {
    expect(manualInstall(UA.iphoneLine, 5)).toBe("ios-in-app");
    expect(manualInstall(UA.iphoneInstagram, 5)).toBe("ios-in-app");
    expect(manualInstall(UA.iphoneWebView, 5)).toBe("ios-in-app");
  });

  it("points Chromium browsers and Android to the browser's menu", () => {
    expect(manualInstall(UA.androidChrome, 5)).toBe("menu");
    expect(manualInstall(UA.androidFirefox, 5)).toBe("menu");
    expect(manualInstall(UA.macChrome, 0)).toBe("menu");
    expect(manualInstall(UA.windowsEdge, 0)).toBe("menu");
  });

  it("knows Safari on a Mac adds to the Dock", () => {
    expect(manualInstall(UA.macSafari, 0)).toBe("mac-dock");
  });

  it("offers nothing where nothing can be installed", () => {
    expect(manualInstall(UA.androidLine, 5)).toBe("none");
    expect(manualInstall(UA.windowsFirefox, 0)).toBe("none");
  });
});
