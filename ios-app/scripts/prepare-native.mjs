import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const app = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const digest = path => createHash('sha256').update(readFileSync(path)).digest('hex');

export function prepareNative(appRoot = app, check = false) {
  const repo = resolve(appRoot, '..');
  const built = join(repo, 'pheme-va/target/ios/whisper');
  const framework = join(built, 'PhemeVA.xcframework');
  const metadataPath = join(built, 'build-metadata.txt');
  if (!existsSync(metadataPath) || !existsSync(join(framework, 'Info.plist'))) {
    throw new Error('Build production Pheme first: cd ../pheme-va && bash scripts/build-ios.sh whisper');
  }
  const metadata = readFileSync(metadataPath, 'utf8');
  if (!/^variant=whisper$/m.test(metadata)) throw new Error('Only the production Whisper XCFramework can be linked.');
  const header = join(repo, 'pheme-va/include/pheme_va.h');
  const headerHash = digest(header);
  const lockHash = digest(join(repo, 'pheme-va/Cargo.lock'));
  if (!metadata.includes(`${headerHash}  include/pheme_va.h`) || !metadata.includes(`${lockHash}  Cargo.lock`)) {
    throw new Error('Pheme headers/Cargo.lock changed. Rebuild the XCFramework before preparing Expo.');
  }
  const slices = [['ios-arm64', 'aarch64-apple-ios'], ['ios-arm64-simulator', 'aarch64-apple-ios-sim']];
  const hashes = {};
  for (const [slice, target] of slices) {
    const symbols = readFileSync(join(built, `${target}-symbols.txt`), 'utf8');
    if (/_pheme_va_test_/.test(symbols) || !/_pheme_va_whisper_new(?:\s|$)/m.test(symbols) || !/_pheme_va_abi_version(?:\s|$)/m.test(symbols)) {
      throw new Error(`Invalid production symbols for ${target}.`);
    }
    const hash = digest(join(framework, slice, 'libffi.a'));
    if (!metadata.split('\n').some(line => line.startsWith(`${hash}  `) && line.endsWith(`/${target}/release/libffi.a`))) {
      throw new Error(`Packaged library does not match build metadata: ${target}`);
    }
    if (digest(join(framework, slice, 'Headers/pheme_va.h')) !== headerHash) throw new Error('Packaged header mismatch.');
    hashes[slice] = hash;
  }
  const source = join(repo, 'pheme-va/hosts/ios/Sources');
  const output = join(appRoot, 'modules/pheme-native/ios');
  const files = ['BridgeTypes.swift', 'NativeBridge.swift'];
  if (check) {
    for (const name of files) {
      if (!existsSync(join(output, 'Generated', name)) || digest(join(output, 'Generated', name)) !== digest(join(source, name))) {
        throw new Error('Shared Swift bridge is stale. Run npm run native:prepare.');
      }
    }
    for (const [slice] of slices) {
      if (digest(join(output, 'Frameworks/PhemeVA.xcframework', slice, 'libffi.a')) !== hashes[slice] ||
          digest(join(output, 'Frameworks/PhemeVA.xcframework', slice, 'Headers/pheme_va.h')) !== headerHash) {
        throw new Error('Prepared Expo framework is stale. Run npm run native:prepare.');
      }
    }
    return;
  }
  mkdirSync(join(output, 'Generated'), { recursive: true });
  mkdirSync(join(output, 'Frameworks'), { recursive: true });
  // Only remove this script's generated destination, never source/build artifacts.
  rmSync(join(output, 'Frameworks/PhemeVA.xcframework'), { recursive: true, force: true });
  cpSync(framework, join(output, 'Frameworks/PhemeVA.xcframework'), { recursive: true });
  for (const name of files) cpSync(join(source, name), join(output, 'Generated', name));
  writeFileSync(join(output, 'Generated/manifest.json'), JSON.stringify({ headerHash, lockHash, libraries: hashes }, null, 2) + '\n');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { prepareNative(app, process.argv.includes('--check')); console.log('Production Pheme framework and shared bridge verified.'); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
