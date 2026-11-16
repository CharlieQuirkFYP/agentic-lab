#!/usr/bin/env node
// Physical-device transport only: inference and evidence are produced by the app.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, realpathSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';

const usage = `Usage:
  node scripts/device-benchmark.mjs inspect --device <UDID> --evidence <directory>
  node scripts/device-benchmark.mjs stage --device <UDID> --source <directory> --dataset <manifest.json> --evidence <directory>
  node scripts/device-benchmark.mjs launch --device <UDID> --id <run-id> --dataset <manifest.json> --asr <id,id> --reasoning <id,id> --evidence <directory> [--repetitions 2] [--cooldown-ms 250] [--seed 1] [--sync paused|live] [--action run|resume|export] [--sample-resources 1|0]
  node scripts/device-benchmark.mjs collect --device <UDID> --id <run-id> --evidence <directory>

Optional --bundle defaults to com.example.incidentreportingagent. Acceptance
defaults to paused uploads; --action export --sync live syncs without inference.
The app must already be installed from a validated Release build. Stage accepts
only a local dataset manifest and WAV assets; it never uploads fixture metrics.
Every operation checks that the selected device is physical. Commands terminate
after a bounded timeout and retain their raw output in the evidence directory.
Collect may run during inference to retrieve durable progress. It does not mark
an incomplete or failed phone execution as passed.`;

function parse(argv) {
  const [operation, ...rest] = argv;
  if (!operation || operation === '--help' || operation === '-h') return { operation: 'help' };
  if (!['inspect', 'stage', 'launch', 'collect'].includes(operation)) throw new Error(`Unknown operation: ${operation}`);
  const allowed = new Set(['device', 'evidence', 'source', 'dataset', 'bundle', 'id', 'asr', 'reasoning', 'repetitions', 'cooldown-ms', 'seed', 'sync', 'action', 'sample-resources']);
  const options = { operation };
  for (let i = 0; i < rest.length; i += 2) {
    const key = rest[i]?.replace(/^--/, '');
    if (!rest[i]?.startsWith('--') || !allowed.has(key) || !rest[i + 1] || rest[i + 1].startsWith('--')) throw new Error(`Invalid option: ${rest[i] ?? ''}`);
    if (Object.hasOwn(options, key)) throw new Error(`Duplicate option: --${key}`);
    options[key] = rest[i + 1];
  }
  for (const key of ['device', 'evidence']) if (!options[key]) throw new Error(`Missing --${key}`);
  return options;
}

function simpleName(value, label) {
  if (!value || !/^[a-zA-Z0-9][a-zA-Z0-9_.-]*$/.test(value)) throw new Error(`Invalid ${label}`);
  return value;
}

function boundedInteger(value, fallback, min, max, label) {
  const number = value === undefined ? fallback : Number(value);
  if (!Number.isSafeInteger(number) || number < min || number > max) throw new Error(`Invalid ${label}: expected ${min}–${max}`);
  return number;
}

function validateDataset(directory, filename) {
  simpleName(filename, 'dataset filename');
  const manifest = JSON.parse(readFileSync(join(directory, filename), 'utf8'));
  if (manifest.schema_version !== 1 || !manifest.dataset_id || !manifest.version || !Array.isArray(manifest.cases) || manifest.cases.length < 1 || manifest.cases.length > 10) throw new Error('Invalid dataset manifest');
  const audio = [];
  for (const item of manifest.cases) {
    if (!item.id || !Array.isArray(item.turns) || item.turns.length < 1 || item.turns.length > 8) throw new Error('Invalid dataset case');
    for (const turn of item.turns) {
      if (!turn.id || typeof turn.audio_filename !== 'string' || !turn.audio_filename.endsWith('.wav')) throw new Error('Each turn needs an ID and local WAV filename');
      const path = realpathSync(resolve(directory, turn.audio_filename));
      const local = relative(directory, path);
      if (!local || local.startsWith(`..${sep}`) || local === '..' || resolve(path) === directory) throw new Error('WAV path escapes the dataset directory');
      const size = statSync(path).size;
      if (size < 44 || size > 32 * 1024 * 1024) throw new Error('WAV exceeds the native input bounds');
      const bytes = readFileSync(path);
      if (bytes.toString('ascii', 0, 4) !== 'RIFF' || bytes.toString('ascii', 8, 12) !== 'WAVE') throw new Error('Expected a RIFF/WAVE recording');
      audio.push({ case_id: item.id, turn_id: turn.id, filename: turn.audio_filename, bytes: size, sha256: createHash('sha256').update(bytes).digest('hex') });
    }
  }
  return { dataset_id: manifest.dataset_id, version: manifest.version, manifest_sha256: createHash('sha256').update(readFileSync(join(directory, filename))).digest('hex'), audio };
}

function main() {
  const options = parse(process.argv.slice(2));
  if (options.operation === 'help') { console.log(usage); return; }
  const evidence = resolve(options.evidence);
  mkdirSync(evidence, { recursive: true });
  const stamp = new Date().toISOString().replace(/[:.]/g, '-');
  const bundle = options.bundle ?? 'com.example.incidentreportingagent';
  let sequence = 0;
  function deviceCommand(label, args, timeoutSeconds = 60) {
    const prefix = join(evidence, `${stamp}-${++sequence}-${label}`);
    const argv = ['devicectl', ...args, '--timeout', String(timeoutSeconds), '--json-output', `${prefix}.json`];
    const result = spawnSync('xcrun', argv, { encoding: 'utf8', timeout: (timeoutSeconds + 15) * 1000, killSignal: 'SIGKILL', maxBuffer: 16 * 1024 * 1024 });
    writeFileSync(`${prefix}.log`, `${result.stdout ?? ''}${result.stderr ?? ''}`);
    writeFileSync(`${prefix}.command.json`, JSON.stringify({ captured_at: new Date().toISOString(), executable: 'xcrun', arguments: argv, status: result.status, signal: result.signal, error: result.error?.message ?? null }, null, 2) + '\n');
    if (result.error || result.status !== 0) throw new Error(`${label} failed; inspect ${prefix}.log`);
    return existsSync(`${prefix}.json`) ? JSON.parse(readFileSync(`${prefix}.json`, 'utf8')) : null;
  }

  const inventory = deviceCommand('devices', ['list', 'devices'], 20);
  const device = inventory?.result?.devices?.find(item => [item.identifier, item.properties?.hardware?.udid, item.hardwareProperties?.udid].includes(options.device));
  if (!device || (device.properties?.hardware?.reality ?? device.hardwareProperties?.reality) !== 'physical') throw new Error('The selected device is not a physical phone');
  const pairing = device.properties?.connection?.pairingState ?? device.connectionProperties?.pairingState;
  if (pairing !== 'paired') throw new Error(`Physical device is not paired (${pairing ?? 'unknown'})`);
  // CoreDevice can report a paired phone as connecting/available until the first
  // operation establishes its tunnel. The bounded operation verifies reachability.
  const deviceID = device.identifier;
  if (options.operation === 'inspect') {
    deviceCommand('lock-state', ['device', 'info', 'lockState', '--device', deviceID], 20);
    console.log(`Physical device verified; raw evidence: ${evidence}`);
    return;
  }
  const domain = ['--device', deviceID, '--domain-type', 'appDataContainer', '--domain-identifier', bundle];
  if (options.operation === 'stage') {
    if (!options.source) throw new Error('Missing --source');
    const source = realpathSync(resolve(options.source));
    const manifest = validateDataset(source, options.dataset);
    writeFileSync(join(evidence, `${stamp}-staged-input.json`), JSON.stringify(manifest, null, 2) + '\n');
    // Copy only declared files, never unrelated exports/metrics from the source directory.
    const staged = join(evidence, `${stamp}-input`);
    for (const filename of new Set([options.dataset, ...manifest.audio.map(item => item.filename)])) {
      mkdirSync(dirname(join(staged, filename)), { recursive: true });
      copyFileSync(join(source, filename), join(staged, filename));
    }
    deviceCommand('stage', ['device', 'copy', 'to', ...domain, '--source', staged, '--destination', 'Documents/BenchmarkAcceptance']);
    console.log(`Staged ${manifest.audio.length} WAV turns from dataset ${manifest.dataset_id}`);
    return;
  }
  const id = simpleName(options.id, 'run ID');
  if (!/^[a-zA-Z0-9-]+$/.test(id)) throw new Error('Run ID must contain only letters, numbers and dashes');
  if (options.operation === 'launch') {
    const dataset = simpleName(options.dataset, 'dataset filename');
    const params = new URLSearchParams({ acceptance: '1', id, dataset });
    const action = options.action ?? 'run';
    const sync = options.sync ?? 'paused';
    if (!['run', 'resume', 'export'].includes(action)) throw new Error('Invalid action');
    if (!['paused', 'live'].includes(sync)) throw new Error('Invalid sync mode');
    params.set('action', action);
    params.set('sync', sync);
    params.set('sample_resources', String(boundedInteger(options['sample-resources'], 1, 0, 1, 'resource sampler mode')));
    for (const key of ['asr', 'reasoning']) {
      if (!options[key]) throw new Error(`Missing --${key}`);
      for (const model of options[key].split(',')) simpleName(model, `${key} model ID`);
      params.set(key, options[key]);
    }
    params.set('repetitions', String(boundedInteger(options.repetitions, 2, 1, 3, 'repetitions')));
    params.set('cooldown_ms', String(boundedInteger(options['cooldown-ms'], 250, 0, 60000, 'cooldown')));
    params.set('seed', String(boundedInteger(options.seed, 1, 0, 2147483647, 'seed')));
    const url = `pheme://benchmark?${params}`;
    deviceCommand('launch', ['device', 'process', 'launch', '--device', deviceID, '--terminate-existing', '--payload-url', url, bundle]);
    console.log(`Launched physical-phone run ${id}; collect its durable report to determine outcome.`);
    return;
  }
  const destination = join(evidence, `${id}-${stamp}`);
  deviceCommand('collect', ['device', 'copy', 'from', ...domain, '--source', `Documents/BenchmarkAcceptance/${id}`, '--destination', destination]);
  console.log(`Copied physical-phone evidence to ${destination}`);
}

try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
