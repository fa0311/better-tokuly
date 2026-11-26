package app.tokuly.webdata

import android.webkit.CookieManager
import android.webkit.WebStorage
import android.webkit.WebView
import expo.modules.kotlin.Promise
import expo.modules.kotlin.functions.Queues
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

/**
 * Removes what the app's web views stored: cookies (in memory and on disk),
 * DOM storage of every origin and the HTTP cache. All three are shared by
 * every WebView of the app, so this is the whole of it.
 */
class TokulyWebDataModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("TokulyWebData")

    // WebView APIs belong to the main thread.
    AsyncFunction("clearAll") { promise: Promise ->
      WebStorage.getInstance().deleteAllData()
      appContext.reactContext?.let { context ->
        // The cache is the app's, not the view's: any WebView clears it.
        val view = WebView(context)
        view.clearCache(true)
        view.destroy()
      }
      val cookies = CookieManager.getInstance()
      cookies.removeAllCookies { _ ->
        cookies.flush()
        promise.resolve(null)
      }
    }.runOnQueue(Queues.MAIN)
  }
}
