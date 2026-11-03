import { createHash } from 'node:crypto';
import { closeSync, cpSync, existsSync, mkdirSync, openSync, readFileSync, readSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const app = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const digest = path => {
  const hash = createHash('sha256'), buffer = Buffer.alloc(1024 * 1024), fd = openSync(path, 'r');
  try { let size; while ((size = readSync(fd, buffer)) > 0) hash.update(buffer.subarray(0, size)); }
  finally { closeSync(fd); }
  return hash.digest('hex');
};
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

export function prepareNative(appRoot = app, check = false, bundleModels) {
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
    const manifest = readFileSync(join(root, 'models/manifest.toml'), 'utf8').split(/^\[\[models\]\]\s*$/m);
    const catalog = [];
    const files = [];
    for (const [id, purpose] of [['whisper-large-v3-turbo', 'transcription'], ['qwen2.5-1.5b-instruct-q4-k-m', 'reasoning']]) {
      const block = manifest.find(text => text.includes(`id = "${id}"`));
      if (!block) throw new Error(`Missing pinned model: ${id}`);
      const filename = block.match(/^model = "([^"]+)"$/m)[1];
      const sha256 = block.match(/^sha256 = "([a-f0-9]{64})"$/m)[1];
      const bytes = Number(block.match(/^model_size_bytes = (\d+)$/m)[1]);
      const source = join(root, 'models', filename);
      const name = filename.split('/').at(-1);
      const destination = join(resource, name);
      const artifact = check ? destination : source;
      if (!existsSync(artifact) || statSync(artifact).size !== bytes || digest(artifact) !== sha256) {
        throw new Error(`Missing or damaged ${id}. Run npm run native:bundle to download/verify and package weights.`);
      }
      catalog.push({ model: { id: sha256, name, sha256, bytes }, purpose, relativePath: name });
      files.push([source, destination]);
    }
    const role = readFileSync(join(root, 'roles/incident-reporting.txt'));
    const catalogJSON = JSON.stringify(catalog, null, 2) + '\n';
    if (check) {
      if (readFileSync(join(resource, 'catalog.json'), 'utf8') !== catalogJSON ||
          !readFileSync(join(resource, 'incident-reporting.txt')).equals(role)) throw new Error('Bundled catalog/role is stale.');
      if (!existsSync(join(resource, 'MODEL-NOTICES.txt'))) throw new Error('Missing model notices.');
    } else {
      rmSync(resource, { recursive: true, force: true });
      mkdirSync(resource, { recursive: true });
      for (const [source, destination] of files) cpSync(source, destination);
      writeFileSync(join(resource, 'catalog.json'), catalogJSON);
      writeFileSync(join(resource, 'incident-reporting.txt'), role);
      writeFileSync(join(resource, 'MODEL-NOTICES.txt'),
        'Development baselines, not validated phone selections.\nWhisper: ggerganov/whisper.cpp, MIT.\nQwen2.5-1.5B-Instruct: Qwen, Apache-2.0.\nSee the repository model manifest for pinned source revisions and checksums.\nInclude full upstream licences before redistribution.\n');
    }
  } else if (check && existsSync(resource)) {
    throw new Error('Unexpected model resources in a weight-free build.');
  } else if (!check) {
    rmSync(resource, { recursive: true, force: true });
  }
  if (!check) {
    // Remove old copied Swift sources; only module-owned sources are compiled.
    rmSync(join(output, 'Generated'), { recursive: true, force: true });
    writeFileSync(prepared, JSON.stringify({ bundledModels: includeModels }) + '\n');
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    prepareNative(app, process.argv.includes('--check'),
      process.argv.includes('--bundle-models') ? true : process.argv.includes('--without-models') ? false : undefined);
    console.log('Expo Pheme transcription/reasoning frameworks and resource selection verified.');
  }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
