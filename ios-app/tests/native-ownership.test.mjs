import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import test from 'node:test';

const app = new URL('../', import.meta.url);
test('Expo is the only iPhone frontend and compiles the module-owned bridges', () => {
  assert.equal(existsSync(new URL('../pheme-va/hosts/ios', app)), false);
  const native = new URL('modules/pheme-native/ios/', app);
  const swift = readdirSync(native).filter(name => name.endsWith('.swift'))
    .map(name => readFileSync(new URL(name, native), 'utf8')).join('\n');
  assert.doesNotMatch(swift, /import SwiftUI|@main|ObservableObject/);
  assert.equal((swift.match(/struct BridgeConfiguration\b/g) ?? []).length, 1);
  assert.equal((swift.match(/struct Transcription\b/g) ?? []).length, 1);
  const spec = readFileSync(new URL('PhemeNative.podspec', native), 'utf8');
  assert.doesNotMatch(spec, /Generated\/|hosts\/ios/);
  assert.match(spec, /PhemeVA\.xcframework/);
  assert.match(spec, /PhemeReasoning\.xcframework/);
  assert.match(spec, /resource_bundles/);
  assert.match(readFileSync(new URL('app/index.tsx', app), 'utf8'), /from "\.\.\/src\/onDeviceChat"/);
  assert.doesNotMatch(readFileSync(new URL('src/localChat.ts', app), 'utf8'), /\bfetch\s*\(/);
});

test('one root collector owns the drain and Release retains the metrics network permission', () => {
  const layout = readFileSync(new URL('app/_layout.tsx', app), 'utf8');
  const settings = readFileSync(new URL('app/local-transcription.tsx', app), 'utf8');
  assert.match(layout, /useEffect\(startMetricsSync/);
  assert.doesNotMatch(settings, /Pheme\.poll\(/);
  assert.match(settings, /useSyncExternalStore\(Metrics\.subscribe/);
  const config = JSON.parse(readFileSync(new URL('app.json', app), 'utf8')).expo.ios.infoPlist;
  assert.match(config.NSLocalNetworkUsageDescription, /metrics/);
  assert.doesNotMatch(config.NSLocalNetworkUsageDescription, /Expo Dev Launcher/);
  assert.equal(config.NSAppTransportSecurity.NSAllowsLocalNetworking, true);
  assert.equal(config.NSAppTransportSecurity.NSAllowsArbitraryLoads, false);
});
