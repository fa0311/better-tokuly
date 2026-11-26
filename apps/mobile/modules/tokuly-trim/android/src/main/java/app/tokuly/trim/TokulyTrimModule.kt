package app.tokuly.trim

import android.net.Uri
import androidx.media3.common.MediaItem
import androidx.media3.transformer.Composition
import androidx.media3.transformer.EditedMediaItem
import androidx.media3.transformer.ExportException
import androidx.media3.transformer.ExportResult
import androidx.media3.transformer.Transformer
import expo.modules.kotlin.Promise
import expo.modules.kotlin.exception.Exceptions
import expo.modules.kotlin.functions.Queues
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import java.io.File
import java.util.UUID

/**
 * Cuts a video down to [startMs, endMs) with Media3 Transformer. The output
 * is an MP4 in the cache directory; its file URI is what the promise gives.
 */
class TokulyTrimModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("TokulyTrim")

    // Transformer works on a thread with a Looper: the main one. The export itself runs on its own threads.
    AsyncFunction("trim") { uri: String, startMs: Double, endMs: Double, promise: Promise ->
      val context = appContext.reactContext ?: throw Exceptions.ReactContextLost()
      val output = File(context.cacheDir, "trim-${UUID.randomUUID()}.mp4")
      val clipping = MediaItem.ClippingConfiguration.Builder()
        .setStartPositionMs(startMs.toLong())
        .setEndPositionMs(endMs.toLong())
        .build()
      val item = MediaItem.Builder()
        .setUri(Uri.parse(uri))
        .setClippingConfiguration(clipping)
        .build()
      val transformer = Transformer.Builder(context)
        .addListener(object : Transformer.Listener {
          override fun onCompleted(composition: Composition, exportResult: ExportResult) {
            promise.resolve(Uri.fromFile(output).toString())
          }

          override fun onError(
            composition: Composition,
            exportResult: ExportResult,
            exportException: ExportException
          ) {
            output.delete()
            promise.reject("ERR_TRIM", exportException.message ?: "trim failed", exportException)
          }
        })
        .build()
      transformer.start(EditedMediaItem.Builder(item).build(), output.absolutePath)
    }.runOnQueue(Queues.MAIN)
  }
}
