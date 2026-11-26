import ExpoModulesCore
import WebKit

/// Removes what web views stored in the app's default website data store:
/// cookies, storage, caches, every data type. (The web screens' views are
/// incognito, with stores of their own that are never written; this clears
/// anything else that ever used the default one.)
public class TokulyWebDataModule: Module {
  public func definition() -> ModuleDefinition {
    Name("TokulyWebData")

    AsyncFunction("clearAll") { (promise: Promise) in
      WKWebsiteDataStore.default().removeData(
        ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(),
        modifiedSince: Date(timeIntervalSince1970: 0)
      ) {
        promise.resolve(nil)
      }
    }.runOnQueue(.main)
  }
}
