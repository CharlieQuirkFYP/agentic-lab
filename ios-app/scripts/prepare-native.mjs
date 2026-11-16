import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { digest, parseModelOptions, preparedSelection, prepareModelResources } from './model-bundle.mjs';

const app = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const slices = [['ios-arm64', 'aarch64-apple-ios'], ['ios-arm64-simulator', 'aarch64-apple-ios-sim']];

function sourceFiles(root, directory = 'crates') {
  return readdirSync(join(root, directory), { withFileTypes: true }).flatMap(entry => {
    const path = `${directory}/${entry.name}`;
    return entry.isDirectory() ? sourceFiles(root, path) : entry.name.endsWith('.rs') || entry.name === 'Cargo.toml' ? [path] : [];
  });
}

function verifySources(root, metadata, header) {
  for (const file of ['Cargo.lock', 'Cargo.toml', `include/${header}`, ...sourceFiles(root)]) {
    if (!metadata.includes(`${digest(join(root, file))}  ${file}\n`)) {
      throw new Error(`Pheme build inputs changed (${file}). Rebuild both XCFrameworks with npm run native:build.`);
    }
  }
}

export function prepareNative(appRoot = app, check = false, bundleModels, modelSelection = preparedSelection(appRoot)) {
  const repo = resolve(appRoot, '..');
  const root = join(repo, 'pheme-va');
  const built = join(root, 'target/ios/whisper');
  const framework = join(built, 'PhemeVA.xcframework');
  const metadataPath = join(built, 'build-metadata.txt');
  if (!existsSync(metadataPath) || !existsSync(join(framework, 'Info.plist'))) {
    throw new Error('Build production Pheme first: npm run native:build');
  }
  const metadata = readFileSync(metadataPath, 'utf8');
  if (!/^variant=whisper$/m.test(metadata)) throw new Error('Only the production Whisper XCFramework can be linked.');
  verifySources(root, metadata, 'pheme_va.h');
  const headerHash = digest(join(root, 'include/pheme_va.h'));
  const replyBuilt = join(root, 'target/ios/reply');
  const replyFramework = join(replyBuilt, 'PhemeReasoning.xcframework');
  if (!existsSync(join(replyBuilt, 'build-metadata.txt')) || !existsSync(join(replyFramework, 'Info.plist'))) {
    throw new Error('Build production reasoning first: npm run native:build');
  }
  verifySources(root, readFileSync(join(replyBuilt, 'build-metadata.txt'), 'utf8'), 'pheme_reply.h');
  const replyHeaderHash = digest(join(root, 'include/pheme_reply.h'));
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
    const exports = readFileSync(join(replyBuilt, `${target}-exports.txt`), 'utf8');
    for (const name of ['create', 'inspect', 'configure', 'respond', 'reset', 'cancel', 'free', 'string_free']) {
      if (!new RegExp(`_pheme_reply_${name}(?:\\s|$)`, 'm').test(exports)) throw new Error(`Missing reasoning C ABI: ${name}`);
    }
    if (/(?:^|\s)_(?:ggml_|gguf_|llama_|pheme_va_test_)/m.test(exports)) throw new Error('Reasoning framework leaks private symbols.');
    const replyHash = digest(join(replyFramework, slice, 'PhemeReasoning.framework/PhemeReasoning'));
    if (!readFileSync(join(replyBuilt, `${target}-sha256.txt`), 'utf8').startsWith(`${replyHash}  `)) {
      throw new Error(`Reasoning library does not match build metadata: ${target}`);
    }
    if (digest(join(replyFramework, slice, 'PhemeReasoning.framework/Headers/pheme_reply.h')) !== replyHeaderHash) {
      throw new Error('Packaged reasoning header mismatch.');
    }
  }
  const output = join(appRoot, 'modules/pheme-native/ios');
  const prepared = join(output, 'Frameworks/prepared.json');
  const includeModels = bundleModels ?? (existsSync(prepared) && JSON.parse(readFileSync(prepared, 'utf8')).bundledModels === true);
  for (const source of [framework, replyFramework]) {
    const destination = join(output, 'Frameworks', source.split('/').at(-1));
    if (check) {
      const compare = (a, b) => {
        for (const entry of readdirSync(a, { withFileTypes: true })) {
          if (entry.isDirectory()) compare(join(a, entry.name), join(b, entry.name));
          else if (!existsSync(join(b, entry.name)) || digest(join(a, entry.name)) !== digest(join(b, entry.name))) {
            throw new Error('Prepared Expo framework is stale. Run npm run native:prepare.');
          }
        }
      };
      compare(source, destination);
    } else {
      mkdirSync(dirname(destination), { recursive: true });
      rmSync(destination, { recursive: true, force: true });
      cpSync(source, destination, { recursive: true });
    }
  }
  const resource = join(output, 'Resources/PhemeModels');
  if (includeModels) {
    prepareModelResources(root, resource, modelSelection, check);
  } else if (check && existsSync(resource)) {
    throw new Error('Unexpected model resources in a weight-free build.');
  } else if (!check) {
    rmSync(resource, { recursive: true, force: true });
  }
  if (!check) {
    // Remove old copied Swift sources; only module-owned sources are compiled.
    rmSync(join(output, 'Generated'), { recursive: true, force: true });
    writeFileSync(prepared, JSON.stringify({ bundledModels: includeModels, ...(includeModels ? { modelSelection } : {}) }) + '\n');
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const options = parseModelOptions(process.argv.slice(2), process.env, preparedSelection(app));
    if (options.download) throw new Error('Use npm run native:bundle to download models.');
    prepareNative(app, options.check, options.bundleModels, options.selection);
    console.log('Expo Pheme transcription/reasoning frameworks and resource selection verified.');
  }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
