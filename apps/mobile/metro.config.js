const { getDefaultConfig } = require("expo/metro-config");
const { withNativeWind } = require("nativewind/metro");

// The seam registries (which feature plugs in where) are written before anything is bundled.
require("./scripts/seams").writeRegistries();

const config = getDefaultConfig(__dirname);

// The shared packages are TypeScript sources that import their siblings as `./x.js`
// (ESM style); there is only `x.ts` on disk.
const resolveRequest = config.resolver.resolveRequest;
config.resolver.resolveRequest = (context, moduleName, platform) => {
  const resolve = resolveRequest ?? context.resolveRequest;
  if (moduleName.startsWith(".") && moduleName.endsWith(".js")) {
    try {
      return resolve(context, moduleName.slice(0, -3), platform);
    } catch {
      // Not a TypeScript source: resolve the name as written.
    }
  }
  return resolve(context, moduleName, platform);
};

module.exports = withNativeWind(config, { input: "./global.css" });
