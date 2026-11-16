import { createHash } from 'node:crypto';
import { closeSync, cpSync, existsSync, mkdirSync, openSync, readFileSync, readSync, readdirSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve, sep } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export const defaultModelSelection = Object.freeze({
  transcription: ['whisper-large-v3-turbo'],
  reasoning: ['qwen3-4b-instruct-2507-q4-k-m'],
});
const idPattern = /^[a-z0-9][a-z0-9.-]{0,95}$/;
const hashPattern = /^[a-f0-9]{64}$/;
const json = value => JSON.stringify(value, null, 2) + '\n';

export function digest(path) {
  const hash = createHash('sha256'), buffer = Buffer.alloc(1024 * 1024), fd = openSync(path, 'r');
  try { let size; while ((size = readSync(fd, buffer)) > 0) hash.update(buffer.subarray(0, size)); }
  finally { closeSync(fd); }
  return hash.digest('hex');
}

export function validateSelection(selection) {
  if (!selection || Object.keys(selection).sort().join(',') !== 'reasoning,transcription') throw new Error('Expected transcription and reasoning model arrays.');
  for (const [purpose, ids] of Object.entries(selection)) {
    if (!Array.isArray(ids) || ids.length < 1 || ids.length > 8 ||
        ids.some(id => typeof id !== 'string' || !idPattern.test(id)) || new Set(ids).size !== ids.length) {
      throw new Error(`Select 1–8 distinct manifest model IDs for ${purpose}.`);
    }
  }
  return selection;
}

// Explicit flags take precedence over environment arrays; unmentioned purposes retain
// their prior prepared selection, or the development defaults in a fresh checkout.
export function parseModelOptions(args, env = process.env, previous = defaultModelSelection) {
  const selection = { ...previous };
  const options = { selection, check: false, bundleModels: undefined, download: false };
  for (const [key, variable] of [['transcription', 'PHEME_IOS_ASR_MODELS'], ['reasoning', 'PHEME_IOS_REASONING_MODELS']]) {
    if (env[variable] !== undefined) { selection[key] = env[variable].split(','); options.bundleModels = true; }
  }
  const seen = new Set();
  for (let index = 0; index < args.length; index++) {
    const argument = args[index];
    if (seen.has(argument)) throw new Error(`Duplicate option: ${argument}`);
    seen.add(argument);
    if (argument === '--asr-models' || argument === '--reasoning-models') {
      const value = args[++index];
      if (!value || value.startsWith('--')) throw new Error(`Missing model IDs for ${argument}`);
      selection[argument === '--asr-models' ? 'transcription' : 'reasoning'] = value.split(',');
      options.bundleModels = true;
    } else if (argument === '--check') options.check = true;
    else if (argument === '--bundle-models') options.bundleModels = true;
    else if (argument === '--without-models') options.bundleModels = false;
    else if (argument === '--download') options.download = true;
    else throw new Error(`Unknown option: ${argument}`);
  }
  if (seen.has('--without-models') && (options.download || options.bundleModels !== false ||
      seen.has('--bundle-models') || seen.has('--asr-models') || seen.has('--reasoning-models') ||
      env.PHEME_IOS_ASR_MODELS !== undefined || env.PHEME_IOS_REASONING_MODELS !== undefined)) {
    throw new Error('--without-models cannot be combined with model selection.');
  }
  validateSelection(selection);
  return options;
}

export function preparedSelection(app) {
  const path = join(app, 'modules/pheme-native/ios/Frameworks/prepared.json');
  return existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')).modelSelection ?? defaultModelSelection : defaultModelSelection;
}

function safePath(root, path) {
  if (typeof path !== 'string' || !/^[A-Za-z0-9_.-]+(?:\/[A-Za-z0-9_.-]+)*$/.test(path) || path.split('/').some(part => part === '.' || part === '..')) {
    throw new Error(`Invalid manifest artifact path: ${path}`);
  }
  const result = join(root, path);
  if (existsSync(result) && !realpathSync(result).startsWith(realpathSync(root) + sep)) throw new Error(`Artifact escapes model root: ${path}`);
  return result;
}

export function selectedModels(root, selection) {
  validateSelection(selection);
  const blocks = readFileSync(join(root, 'models/manifest.toml'), 'utf8').split(/^\[\[models\]\]\s*$/m).slice(1);
  const manifest = new Map();
  for (const block of blocks) {
    // Packaging consumes the manifest's simple scalar fields only; unsupported
    // arrays/fields stay with the runtime's TOML reader.
    const fields = Object.fromEntries([...block.matchAll(/^(\w+) = ("[^"\n]*"|\d+)\s*$/gm)].map(([, key, value]) => [key, JSON.parse(value)]));
    if (manifest.has(fields.id)) throw new Error(`Duplicate manifest ID: ${fields.id}`);
    manifest.set(fields.id, fields);
  }
  const models = Object.entries(selection).flatMap(([purpose, ids]) => ids.map(id => {
    const entry = manifest.get(id);
    if (!entry) throw new Error(`Missing pinned model: ${id}`);
    if (entry.purpose !== (purpose === 'transcription' ? 'transcript' : 'reply') ||
        entry.runtime !== (purpose === 'transcription' ? 'whispercpp' : 'llama.cpp')) throw new Error(`Unsupported ${purpose} runtime/model: ${id}`);
    if (!idPattern.test(entry.download_id ?? '') || !hashPattern.test(entry.sha256 ?? '') ||
        !Number.isSafeInteger(entry.model_size_bytes) || entry.model_size_bytes <= 0 ||
        !/^[a-zA-Z0-9_.-]+\/[a-zA-Z0-9_.-]+$/.test(entry.repository ?? '') || !/^[a-f0-9]{40}$/.test(entry.revision ?? '')) throw new Error(`Incomplete pinned model metadata: ${id}`);
    if (!['mit', 'apache-2.0'].includes(entry.license) || !hashPattern.test(entry.license_sha256 ?? '') ||
        !/^https:\/\//.test(entry.license_url ?? '')) throw new Error(`Missing verified model license: ${id}`);
    const source = safePath(join(root, 'models'), entry.model);
    const licenseSource = safePath(join(root, 'models'), entry.license_file);
    if (!existsSync(licenseSource) || digest(licenseSource) !== entry.license_sha256) throw new Error(`Missing or damaged model license: ${id}`);
    const name = basename(source);
    if (!(purpose === 'transcription' ? name.endsWith('.bin') : name.endsWith('.gguf'))) throw new Error(`Unsupported model artifact: ${id}`);
    return { ...entry, purpose, source, licenseSource, name, licenseName: basename(licenseSource) };
  }));
  if (new Set(models.map(model => model.name)).size !== models.length || new Set(models.map(model => model.sha256)).size !== models.length) throw new Error('Selected models must have distinct filenames and checksums.');
  return models;
}

export function prepareModelResources(root, resource, selection, check = false) {
  const models = selectedModels(root, selection);
  for (const model of models) {
    const artifact = check ? join(resource, model.name) : model.source;
    if (!existsSync(artifact) || statSync(artifact).size !== model.model_size_bytes || digest(artifact) !== model.sha256) {
      throw new Error(`Missing or damaged ${model.id}. Run npm run native:bundle with the selected model IDs.`);
    }
  }
  const catalog = models.map(model => ({
    model: { id: model.sha256, name: model.name, sha256: model.sha256, bytes: model.model_size_bytes },
    purpose: model.purpose, relativePath: model.name,
  }));
  const resources = new Map([
    ['catalog.json', Buffer.from(json(catalog))],
    ['selection.json', Buffer.from(json(selection))],
    ['incident-reporting.txt', readFileSync(join(root, 'roles/incident-reporting.txt'))],
    ['MODEL-NOTICES.txt', Buffer.from('Development artifacts; physical-device loading and quality require evaluation.\n\n' + models.map(model =>
      `${model.id}\nSource: https://huggingface.co/${model.repository}/tree/${model.revision}\nArtifact: ${model.model}\nSHA-256: ${model.sha256}\nBytes: ${model.model_size_bytes}\nLicense: ${model.license} (${model.licenseName})\nLicense source: ${model.license_url}\n`).join('\n'))],
  ]);
  for (const model of models) resources.set(model.licenseName, readFileSync(model.licenseSource));
  if (check) {
    for (const [name, content] of resources) {
      if (!existsSync(join(resource, name)) || !readFileSync(join(resource, name)).equals(content)) throw new Error(`Bundled resource is missing or stale: ${name}`);
    }
    const expected = [...resources.keys(), ...models.map(model => model.name)].sort();
    // CocoaPods adds Info.plist when it builds PhemeModels.bundle.
    const actual = readdirSync(resource).filter(name => name !== 'Info.plist').sort();
    if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error('Unexpected extra bundled model resources.');
  } else {
    rmSync(resource, { recursive: true, force: true });
    mkdirSync(resource, { recursive: true });
    for (const model of models) cpSync(model.source, join(resource, model.name));
    for (const [name, content] of resources) writeFileSync(join(resource, name), content);
  }
  return models;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const app = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const root = resolve(app, '../pheme-va');
  try {
    if (process.argv[2] === '--check-resource') {
      if (process.argv.length !== 4) throw new Error('Usage: model-bundle.mjs --check-resource BUNDLE');
      const models = prepareModelResources(root, resolve(process.argv[3]), preparedSelection(app), true);
      for (const model of models) console.log(`PASS: ${model.id}, ${model.model_size_bytes} bytes, SHA-256 ${model.sha256}`);
    } else {
      const options = parseModelOptions(process.argv.slice(2), process.env, preparedSelection(app));
      if (!options.download) throw new Error('Use --download to obtain the selected model artifacts.');
      const models = selectedModels(root, options.selection);
      for (const model of models) {
        const result = spawnSync('bash', [join(root, 'scripts/download-model.sh'), model.download_id], {
          stdio: 'inherit', env: { ...process.env, PHEME_VA_MODEL_DIR: join(root, 'models') },
        });
        if (result.error || result.status !== 0) throw new Error(`Download failed for ${model.id}: ${result.error?.message ?? result.status}`);
      }
    }
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
