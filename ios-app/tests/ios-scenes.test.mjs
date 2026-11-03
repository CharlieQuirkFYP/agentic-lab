import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import scenes from '../scripts/with-ios-scenes.cjs';

const legacy = `
class AppDelegate: ExpoAppDelegate {
  var window: UIWindow?
  var reactNativeFactory: RCTReactNativeFactory?
  func start() {
    reactNativeFactory = factory
#if os(iOS) || os(tvOS)
    window = UIWindow(frame: UIScreen.main.bounds)
    factory.startReactNative(
      withModuleName: "main",
      in: window,
      launchOptions: launchOptions)
#endif
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }
  // Linking API
}
`;

test('Expo owns scene startup without starting a second React runtime', () => {
  const migrated = scenes.adoptScenes(legacy);
  assert.match(migrated, /ExpoAppDelegate, ExpoReactNativeFactoryProvider/);
  assert.match(migrated, /reactNativeFactory = factory/);
  assert.match(migrated, /super.application/);
  assert.match(migrated, /Linking API/);
  assert.doesNotMatch(migrated, /startReactNative|UIWindow\(frame:/);
  assert.equal(scenes.adoptScenes(migrated), migrated);
});

test('an unsupported template fails explicitly instead of silently retaining old startup', () => {
  assert.throws(() => scenes.adoptScenes('class AppDelegate {}'), /Unrecognized/);
  assert.throws(() => scenes.adoptScenes(legacy.replace('in: window', 'in: anotherWindow')), /Cannot migrate/);
  assert.throws(() => scenes.adoptScenes(scenes.adoptScenes(legacy) + '\nfactory.startReactNative()'), /owned only/);
});

test('app prebuild registers Expo scenes and keeps a single scene', async () => {
  const app = JSON.parse(readFileSync(new URL('../app.json', import.meta.url), 'utf8'));
  assert.ok(app.expo.plugins.includes('./scripts/with-ios-scenes.cjs'));
  const config = scenes({ name: 'Fixture', slug: 'fixture' });
  const result = await config.mods.ios.infoPlist({
    modResults: {}, modRequest: { nextMod: value => value },
  });
  assert.deepEqual(result.modResults.UIApplicationSceneManifest, {
    UIApplicationSupportsMultipleScenes: false,
    UISceneConfigurations: {
      UIWindowSceneSessionRoleApplication: [{
        UISceneConfigurationName: 'Default Configuration',
        UISceneDelegateClassName: 'EXExpoAppSceneDelegate',
      }],
    },
  });
});
