import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { CameraScreen } from "../camera-screen";

// The camera itself is the edge (the emulator has none): permissions, outputs and the recorder.
const mockCamera = {
  hasPermission: true,
  canRequestPermission: false,
  requestPermission: jest.fn(async () => true),
};
const mockMicrophone = { hasPermission: true, canRequestPermission: false, requestPermission: jest.fn() };
const mockCapture = jest.fn(async () => ({ filePath: "/tmp/photo.jpg" }));
let mockFinish: (path: string) => void = () => {};
const mockRecorder = {
  recordedDuration: 4.2,
  startRecording: jest.fn(async (onFinished: (path: string) => void) => {
    mockFinish = onFinished;
  }),
  stopRecording: jest.fn(async () => mockFinish("/tmp/video.mp4")),
};
const mockCreateRecorder = jest.fn(async () => mockRecorder);

jest.mock("react-native-vision-camera", () => {
  const { View } = require("react-native");
  return {
    Camera: (props: object) => require("react").createElement(View, { testID: "camera-preview", ...props }),
    useCameraPermission: () => mockCamera,
    useMicrophonePermission: () => mockMicrophone,
    useCameraDevice: (position: string) => ({ position, hasFlash: position === "back", hasTorch: true }),
    usePhotoOutput: () => ({ capturePhotoToFile: mockCapture }),
    useVideoOutput: () => ({ createRecorder: mockCreateRecorder }),
  };
});

beforeEach(() => {
  jest.clearAllMocks();
  mockCamera.hasPermission = true;
  mockCamera.canRequestPermission = false;
});

describe("CameraScreen", () => {
  it("explains why before asking for the camera, then the microphone", async () => {
    mockCamera.hasPermission = false;
    mockCamera.canRequestPermission = true;
    mockMicrophone.canRequestPermission = true;
    await render(<CameraScreen limitSeconds={60} photos onCapture={jest.fn()} onClose={jest.fn()} />);
    expect(screen.getByText("カメラをつかってもいい？")).toBeOnTheScreen();
    expect(mockCamera.requestPermission).not.toHaveBeenCalled();
    await fireEvent.press(screen.getByTestId("camera-allow"));
    await waitFor(() => expect(mockMicrophone.requestPermission).toHaveBeenCalled());
    mockMicrophone.canRequestPermission = false;
  });

  it("shows the story limit before anything is recorded, and a tap takes a photo with the flash chosen", async () => {
    const onCapture = jest.fn();
    await render(<CameraScreen limitSeconds={60} photos onCapture={onCapture} onClose={jest.fn()} />);
    expect(screen.getByTestId("camera-limit")).toHaveTextContent("タップで写真、長おしで動画！（60秒まで）");
    await fireEvent.press(screen.getByTestId("camera-flash"));
    await fireEvent.press(screen.getByTestId("camera-shutter"));
    await waitFor(() =>
      expect(onCapture).toHaveBeenCalledWith(
        expect.objectContaining({ uri: "file:///tmp/photo.jpg", kind: "image" }),
      ),
    );
    expect(mockCapture).toHaveBeenCalledWith({ flashMode: "auto" }, {});
  });

  it("records while the shutter is held, stopping at the server's limit", async () => {
    const onCapture = jest.fn();
    await render(<CameraScreen limitSeconds={60} photos onCapture={onCapture} onClose={jest.fn()} />);
    await fireEvent(screen.getByTestId("camera-shutter"), "longPress");
    await waitFor(() => expect(mockRecorder.startRecording).toHaveBeenCalled());
    expect(mockCreateRecorder).toHaveBeenCalledWith({ maxDuration: 60 });
    await fireEvent(screen.getByTestId("camera-shutter"), "pressOut");
    await waitFor(() =>
      expect(onCapture).toHaveBeenCalledWith(
        expect.objectContaining({ uri: "file:///tmp/video.mp4", kind: "video", duration: 4.2 }),
      ),
    );
    expect(mockCapture).not.toHaveBeenCalled();
  });

  it("only records for a short: a tap starts, a tap stops, up to three minutes", async () => {
    const onCapture = jest.fn();
    await render(<CameraScreen limitSeconds={180} photos={false} onCapture={onCapture} onClose={jest.fn()} />);
    expect(screen.getByTestId("camera-limit")).toHaveTextContent("タップで録画スタート！（3分まで）");
    await fireEvent.press(screen.getByTestId("camera-shutter"));
    await waitFor(() => expect(mockCreateRecorder).toHaveBeenCalledWith({ maxDuration: 180 }));
    await fireEvent.press(screen.getByTestId("camera-shutter"));
    await waitFor(() => expect(onCapture).toHaveBeenCalledWith(expect.objectContaining({ kind: "video" })));
  });
});
