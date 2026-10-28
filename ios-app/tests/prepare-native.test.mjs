import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import assert from 'node:assert/strict';
import test from 'node:test';
import { prepareNative } from '../scripts/prepare-native.mjs';

const sha = text => createHash('sha256').update(text).digest('hex');
test('artifact preparation checks production identity, slices and shared source freshness', () => {
  const root = mkdtempSync(join(tmpdir(), 'pheme-package-')); const app = join(root, 'ios-app');
  const put = (path, text) => { mkdirSync(join(root, path, '..'), { recursive: true }); writeFileSync(join(root, path), text); };
  try {
    assert.throws(() => prepareNative(app), /Build production/);
    const built = 'pheme-va/target/ios/whisper';
    put(`${built}/PhemeVA.xcframework/Info.plist`, '<plist/>');
    put('pheme-va/include/pheme_va.h', 'header'); put('pheme-va/Cargo.lock', 'lock');
    put('pheme-va/hosts/ios/Sources/NativeBridge.swift', 'bridge'); put('pheme-va/hosts/ios/Sources/BridgeTypes.swift', 'types');
    const metadata = `variant=whisper\n${sha('header')}  include/pheme_va.h\n${sha('lock')}  Cargo.lock\n` +
      [['ios-arm64', 'aarch64-apple-ios'], ['ios-arm64-simulator', 'aarch64-apple-ios-sim']].map(([slice, target]) => {
        put(`${built}/PhemeVA.xcframework/${slice}/libffi.a`, target);
        put(`${built}/PhemeVA.xcframework/${slice}/Headers/pheme_va.h`, 'header');
        put(`${built}/${target}-symbols.txt`, '_pheme_va_abi_version\n_pheme_va_whisper_new\n');
        return `${sha(target)}  /build/${target}/release/libffi.a\n`;
      }).join('');
    put(`${built}/build-metadata.txt`, metadata);
    prepareNative(app); prepareNative(app, true);
    assert.equal(readFileSync(join(app, 'modules/pheme-native/ios/Generated/NativeBridge.swift'), 'utf8'), 'bridge');
    put('pheme-va/hosts/ios/Sources/NativeBridge.swift', 'changed');
    assert.throws(() => prepareNative(app, true), /stale/);
    prepareNative(app);
    put(`${built}/aarch64-apple-ios-symbols.txt`, '_pheme_va_test_new\n');
    assert.throws(() => prepareNative(app), /production symbols/);
    put(`${built}/build-metadata.txt`, metadata.replace('variant=whisper', 'variant=test'));
    assert.throws(() => prepareNative(app), /production Whisper/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
