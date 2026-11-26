import { AVATAR_FRAME, BANNER_FRAME, framing } from "../profile-image";

jest.mock("expo-image-manipulator", () => ({}));

describe("profile image framing", () => {
  it("scale a huge square avatar down to 640 px", () => {
    expect(framing({ width: 12000, height: 12000 }, AVATAR_FRAME)).toEqual({
      crop: { originX: 0, originY: 0, width: 12000, height: 12000 },
      size: { width: 640, height: 640 },
    });
  });

  it("cut an uncropped banner at the middle to 3:1, 1500 px wide", () => {
    expect(framing({ width: 4000, height: 3000 }, BANNER_FRAME)).toEqual({
      crop: { originX: 0, originY: 833, width: 4000, height: 1333 },
      size: { width: 1500, height: 500 },
    });
  });

  it("never scale a small image up", () => {
    expect(framing({ width: 300, height: 400 }, AVATAR_FRAME)).toEqual({
      crop: { originX: 0, originY: 50, width: 300, height: 300 },
      size: { width: 300, height: 300 },
    });
  });
});
