const { withAppDelegate, withInfoPlist } = require('expo/config-plugins');

// Expo SDK 57 ships the scene delegate, but its prebuild template still starts
// React Native from UIApplicationDelegate. iOS 27 requires scene-owned startup.
function adoptScenes(source) {
  const declaration = /class AppDelegate: ExpoAppDelegate(?:,\s*ExpoReactNativeFactoryProvider)?\s*\{/;
  const startup = /#if os\(iOS\) \|\| os\(tvOS\)\s+window = UIWindow\(frame: UIScreen\.main\.bounds\)\s+factory\.startReactNative\(\s+withModuleName: "main",\s+in: window,\s+launchOptions: launchOptions\)\s+#endif/;
  if (!declaration.test(source) || !source.includes('var reactNativeFactory: RCTReactNativeFactory?')) {
    throw new Error('Unrecognized Expo AppDelegate. Check the SDK scene lifecycle template before building.');
  }
  if (!startup.test(source) && !source.includes('ExpoAppDelegate, ExpoReactNativeFactoryProvider')) {
    throw new Error('Cannot migrate Expo React Native startup to its scene delegate.');
  }
  const migrated = source.replace(startup, '').replace(
    declaration, 'class AppDelegate: ExpoAppDelegate, ExpoReactNativeFactoryProvider {',
  );
  if (migrated.includes('startReactNative(')) {
    throw new Error('React Native startup must be owned only by ExpoAppSceneDelegate.');
  }
  return migrated;
}

function withIOSScenes(config) {
  config = withInfoPlist(config, config => {
    config.modResults.UIApplicationSceneManifest = {
      UIApplicationSupportsMultipleScenes: false,
      UISceneConfigurations: {
        UIWindowSceneSessionRoleApplication: [{
          UISceneConfigurationName: 'Default Configuration',
          UISceneDelegateClassName: 'EXExpoAppSceneDelegate',
        }],
      },
    };
    return config;
  });
  return withAppDelegate(config, config => {
    if (config.modResults.language !== 'swift') {
      throw new Error('The Expo scene plugin requires the Swift AppDelegate template.');
    }
    config.modResults.contents = adoptScenes(config.modResults.contents);
    return config;
  });
}

module.exports = withIOSScenes;
module.exports.adoptScenes = adoptScenes;
