import { notificationPath } from "@tokuly/core/notifications";
import { screenFor } from "#/seams/links";

const web = (path: string) => `/web?path=${encodeURIComponent(path)}`;

describe("community links", () => {
  it("open the app's own community screens", () => {
    expect(screenFor("/communities")).toBe("/communities");
    expect(screenFor("/g/neko")).toBe("/g/neko");
    expect(screenFor("/g/neko/t/bXou6FpLkfK?reply=r1")).toBe("/g/neko/t/bXou6FpLkfK?reply=r1");
    expect(screenFor("/g/neko/members")).toBe("/g/neko/members");
    expect(screenFor("/join/Ab12Cd34Ef")).toBe("/g/join/Ab12Cd34Ef");
  });

  it("leave making one, events and the staff's pages on the web", () => {
    expect(screenFor("/communities/new")).toBe(web("/communities/new"));
    expect(screenFor("/g/neko/events/0199-ab")).toBe(web("/g/neko/events/0199-ab"));
    expect(screenFor("/g/neko/manage/queue")).toBe(web("/g/neko/manage/queue"));
  });

  it("are where community notifications go, and a video channel stays one", () => {
    expect(screenFor(notificationPath("community.reply", "/c/neko/t/bXou6FpLkfK"))).toBe(
      "/g/neko/t/bXou6FpLkfK",
    );
    expect(screenFor(notificationPath("community.join_approved", "/c/neko"))).toBe("/g/neko");
    expect(screenFor(notificationPath("video.upload", "/c/neko"))).toBe("/c/neko");
  });
});
