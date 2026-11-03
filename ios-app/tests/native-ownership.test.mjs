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
