import AVFoundation
import ExpoModulesCore

/// Cuts a video down to [startMs, endMs) with AVAssetExportSession: passthrough
/// (no re-encode) when the asset can be written to MP4 that way, otherwise the
/// highest quality preset. The output is an MP4 in the temporary directory.
public class TokulyTrimModule: Module {
  public func definition() -> ModuleDefinition {
    Name("TokulyTrim")

    AsyncFunction("trim") { (uri: URL, startMs: Double, endMs: Double, promise: Promise) in
      let asset = AVURLAsset(url: uri)
      let output = FileManager.default.temporaryDirectory
        .appendingPathComponent("trim-\(UUID().uuidString).mp4")
      guard let session = Self.exportSession(for: asset) else {
        promise.reject("ERR_TRIM", "this video cannot be exported")
        return
      }
      session.outputURL = output
      session.outputFileType = .mp4
      session.shouldOptimizeForNetworkUse = true
      session.timeRange = CMTimeRange(
        start: CMTime(value: CMTimeValue(startMs), timescale: 1000),
        end: CMTime(value: CMTimeValue(endMs), timescale: 1000)
      )
      session.exportAsynchronously {
        if session.status == .completed {
          promise.resolve(output.absoluteString)
        } else {
          try? FileManager.default.removeItem(at: output)
          promise.reject("ERR_TRIM", session.error?.localizedDescription ?? "trim failed")
        }
      }
    }
  }

  private static func exportSession(for asset: AVAsset) -> AVAssetExportSession? {
    if let passthrough = AVAssetExportSession(asset: asset, presetName: AVAssetExportPresetPassthrough),
      passthrough.supportedFileTypes.contains(.mp4)
    {
      return passthrough
    }
    return AVAssetExportSession(asset: asset, presetName: AVAssetExportPresetHighestQuality)
  }
}
