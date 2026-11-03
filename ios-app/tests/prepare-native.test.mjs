import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import assert from 'node:assert/strict';
import test from 'node:test';
import { prepareNative } from '../scripts/prepare-native.mjs';

const sha = text => createHash('sha256').update(text).digest('hex');
test('Expo packages both production frameworks, rejects stale inputs, and bundles verified models explicitly', () => {
  const root = mkdtempSync(join(tmpdir(), 'pheme-package-')); const app = join(root, 'ios-app');
  const put = (path, text) => { mkdirSync(join(root, path, '..'), { recursive: true }); writeFileSync(join(root, path), text); };
  try {
    assert.throws(() => prepareNative(app), /Build production/);
    const built = 'pheme-va/target/ios/whisper';
    put(`${built}/PhemeVA.xcframework/Info.plist`, '<plist/>');
    put('pheme-va/include/pheme_va.h', 'header'); put('pheme-va/Cargo.lock', 'lock');
    put('pheme-va/Cargo.toml', 'workspace'); put('pheme-va/crates/core/src/lib.rs', 'core');
    const inputs = `${sha('lock')}  Cargo.lock\n${sha('workspace')}  Cargo.toml\n${sha('core')}  crates/core/src/lib.rs\n`;
    const metadata = `variant=whisper\n${sha('header')}  include/pheme_va.h\n${inputs}` +
      [['ios-arm64', 'aarch64-apple-ios'], ['ios-arm64-simulator', 'aarch64-apple-ios-sim']].map(([slice, target]) => {
        put(`${built}/PhemeVA.xcframework/${slice}/libffi.a`, target);
        put(`${built}/PhemeVA.xcframework/${slice}/Headers/pheme_va.h`, 'header');
        put(`${built}/${target}-symbols.txt`, '_pheme_va_abi_version\n_pheme_va_whisper_new\n');
        return `${sha(target)}  /build/${target}/release/libffi.a\n`;
      }).join('');
    put(`${built}/build-metadata.txt`, metadata);
    const reply = 'pheme-va/target/ios/reply';
    put('pheme-va/include/pheme_reply.h', 'reply-header');
    put(`${reply}/build-metadata.txt`, `variant=reply\n${sha('reply-header')}  include/pheme_reply.h\n${inputs}`);
    put(`${reply}/PhemeReasoning.xcframework/Info.plist`, '<plist/>');
    for (const [slice, target] of [['ios-arm64', 'aarch64-apple-ios'], ['ios-arm64-simulator', 'aarch64-apple-ios-sim']]) {
      put(`${reply}/PhemeReasoning.xcframework/${slice}/PhemeReasoning.framework/PhemeReasoning`, 'reply-' + target);
      put(`${reply}/PhemeReasoning.xcframework/${slice}/PhemeReasoning.framework/Headers/pheme_reply.h`, 'reply-header');
      put(`${reply}/${target}-sha256.txt`, `${sha('reply-' + target)}  binary\n`);
      put(`${reply}/${target}-exports.txt`, ['create', 'inspect', 'configure', 'respond', 'reset', 'cancel', 'free', 'string_free'].map(name => `_pheme_reply_${name}`).join('\n') + '\n');
    }
    prepareNative(app); prepareNative(app, true);
    assert.equal(existsSync(join(app, 'modules/pheme-native/ios/Generated')), false);
    assert.equal(existsSync(join(app, 'modules/pheme-native/ios/Resources/PhemeModels')), false);
    assert.equal(readFileSync(join(app, 'modules/pheme-native/ios/Frameworks/PhemeReasoning.xcframework/ios-arm64/PhemeReasoning.framework/PhemeReasoning'), 'utf8'), 'reply-aarch64-apple-ios');
    put('pheme-va/crates/core/src/lib.rs', 'changed');
    assert.throws(() => prepareNative(app, true), /inputs changed/);
    put('pheme-va/crates/core/src/lib.rs', 'core');
    put('ios-app/modules/pheme-native/ios/Frameworks/PhemeVA.xcframework/ios-arm64/libffi.a', 'damaged');
    assert.throws(() => prepareNative(app, true), /stale/);
    prepareNative(app);
    put('pheme-va/models/manifest.toml', [['whisper-large-v3-turbo', 'transcript/test.bin'], ['qwen2.5-1.5b-instruct-q4-k-m', 'reply/test.gguf']].map(([id, path]) =>
      `[[models]]\nid = "${id}"\nmodel = "${path}"\nsha256 = "${sha('abc')}"\nmodel_size_bytes = 3\n`).join('\n'));
    put('pheme-va/models/transcript/test.bin', 'abc'); put('pheme-va/models/reply/test.gguf', 'abc');
    put('pheme-va/roles/incident-reporting.txt', 'Ask, do not invent.');
    prepareNative(app, false, true); prepareNative(app, true);
    const resource = 'ios-app/modules/pheme-native/ios/Resources/PhemeModels';
    assert.equal(JSON.parse(readFileSync(join(root, resource, 'catalog.json'), 'utf8')).length, 2);
    put(`${resource}/test.gguf`, 'bad');
    assert.throws(() => prepareNative(app, true), /damaged/);
    prepareNative(app, false, true);
    prepareNative(app, false, false); prepareNative(app, true);
    assert.equal(existsSync(join(root, resource)), false);
    const exports = readFileSync(join(root, reply, 'aarch64-apple-ios-exports.txt'), 'utf8');
    put(`${reply}/aarch64-apple-ios-exports.txt`, exports + '_ggml_init\n');
    assert.throws(() => prepareNative(app), /private symbols/);
    put(`${reply}/aarch64-apple-ios-exports.txt`, exports);
    put(`${built}/aarch64-apple-ios-symbols.txt`, '_pheme_va_test_new\n');
    assert.throws(() => prepareNative(app), /production symbols/);
    put(`${built}/build-metadata.txt`, metadata.replace('variant=whisper', 'variant=test'));
    assert.throws(() => prepareNative(app), /production Whisper/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
