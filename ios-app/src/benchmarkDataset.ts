/** Immutable, content-addressed local WAV corpus. This module has no native dependencies. */
export interface BenchmarkAudio {
  id: string; uri: string; sha256: string; bytes: number; duration_ms: number;
  sample_rate: number; channels: number; bits_per_sample: 16;
}
export interface BenchmarkCase { id: string; title: string; turns: BenchmarkAudio[] }
export interface BenchmarkDataset {
  schema_version: 1; dataset_id: string; version: number; revision: string;
  source: 'recorded' | 'imported' | 'synthetic_speech' | 'mixed'; cases: BenchmarkCase[];
}
export const DATASET_LIMITS = { cases: 10, turns: 8, duration_seconds: 120, bytes: 200 * 1024 * 1024 };
export function safeIdentifier(value: string): string {
  if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,79}$/.test(value)) throw new Error('Use an identifier of 1–80 letters, numbers, underscores or dashes.');
  return value;
}

/** SHA-256 over bytes, used for corpus identity without a platform crypto dependency. */
export function sha256(bytes: Uint8Array): string {
  const k = [0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2];
  const h = [0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19];
  const data = new Uint8Array(Math.ceil((bytes.length + 9) / 64) * 64); data.set(bytes); data[bytes.length] = 128;
  const view = new DataView(data.buffer); view.setUint32(data.length - 8, Math.floor(bytes.length / 0x20000000)); view.setUint32(data.length - 4, bytes.length * 8);
  const rotr = (x: number, n: number) => (x >>> n) | (x << (32 - n)); const w = new Int32Array(64);
  for (let offset = 0; offset < data.length; offset += 64) {
    for (let i = 0; i < 16; i++) w[i] = view.getInt32(offset + i * 4);
    for (let i = 16; i < 64; i++) { const x = w[i - 15], y = w[i - 2]; w[i] = w[i - 16] + (rotr(x,7)^rotr(x,18)^(x>>>3)) + w[i - 7] + (rotr(y,17)^rotr(y,19)^(y>>>10)); }
    let [a,b,c,d,e,f,g,z] = h;
    for (let i = 0; i < 64; i++) { const t = (z+(rotr(e,6)^rotr(e,11)^rotr(e,25))+((e&f)^(~e&g))+k[i]+w[i])|0; const u = ((rotr(a,2)^rotr(a,13)^rotr(a,22))+((a&b)^(a&c)^(b&c)))|0; z=g;g=f;f=e;e=(d+t)|0;d=c;c=b;b=a;a=(t+u)|0; }
    [a,b,c,d,e,f,g,z].forEach((v,i) => { h[i] = (h[i]+v)|0; });
  }
  return h.map(v => (v >>> 0).toString(16).padStart(8,'0')).join('');
}

export function inspectWav(bytes: Uint8Array): Omit<BenchmarkAudio, 'id' | 'uri'> {
  if (bytes.length < 44 || bytes.length > DATASET_LIMITS.bytes) throw new Error('WAV is empty or too large.');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tag = (at: number) => String.fromCharCode(...bytes.subarray(at, at + 4));
  if (tag(0) !== 'RIFF' || tag(8) !== 'WAVE' || view.getUint32(4,true) + 8 > bytes.length) throw new Error('Import a complete RIFF WAV recording.');
  let rate = 0, channels = 0, block = 0, dataBytes = 0, format = false;
  for (let at = 12; at + 8 <= bytes.length;) {
    const length = view.getUint32(at + 4,true), start = at + 8;
    if (start + length > bytes.length) throw new Error('WAV contains a truncated chunk.');
    if (tag(at) === 'fmt ') {
      if (length < 16 || view.getUint16(start,true) !== 1 || view.getUint16(start+14,true) !== 16) throw new Error('Use uncompressed 16-bit PCM WAV.');
      channels = view.getUint16(start+2,true); rate = view.getUint32(start+4,true); block = view.getUint16(start+12,true); format = true;
    }
    if (tag(at) === 'data') dataBytes += length;
    at = start + length + (length % 2);
  }
  if (!format || channels < 1 || channels > 2 || rate < 8000 || rate > 96000 || block !== channels*2 || !dataBytes || dataBytes % block) throw new Error('WAV has an unsupported or empty audio format.');
  const duration = dataBytes / (rate * block);
  if (duration > DATASET_LIMITS.duration_seconds) throw new Error('Each benchmark utterance must be at most 120 seconds.');
  return { sha256: sha256(bytes), bytes: bytes.length, duration_ms: duration*1000, sample_rate: rate, channels, bits_per_sample: 16 };
}
export function freezeDataset(input: Omit<BenchmarkDataset, 'revision'>): BenchmarkDataset {
  safeIdentifier(input.dataset_id);
  if (input.schema_version !== 1 || !Number.isSafeInteger(input.version) || input.version < 1) throw new Error('Unsupported dataset version.');
  if (!input.cases.length || input.cases.length > DATASET_LIMITS.cases) throw new Error('A dataset needs 1–10 conversations.');
  const ids = new Set<string>(); let bytes = 0;
  for (const entry of input.cases) {
    safeIdentifier(entry.id); if (ids.has(entry.id)) throw new Error('Conversation identifiers must be unique.'); ids.add(entry.id);
    if (!entry.turns.length || entry.turns.length > DATASET_LIMITS.turns) throw new Error('Each conversation needs 1–8 turns.');
    const turns = new Set<string>();
    for (const turn of entry.turns) {
      safeIdentifier(turn.id); if (turns.has(turn.id)) throw new Error('Turn identifiers must be unique within a conversation.'); turns.add(turn.id);
      if (!/^[a-f0-9]{64}$/.test(turn.sha256) || !turn.uri || !Number.isFinite(turn.duration_ms) || turn.duration_ms <= 0 || turn.duration_ms > 120000 || !Number.isSafeInteger(turn.bytes) || turn.bytes <= 0) throw new Error('Invalid pinned audio metadata.');
      bytes += turn.bytes;
    }
  }
  if (bytes > DATASET_LIMITS.bytes) throw new Error('Dataset exceeds the 200 MiB corpus limit.');
  // Local names and paths never change content identity and never enter metrics exports.
  const manifest = { schema_version: 1, dataset_id: input.dataset_id, version: input.version, source: input.source, cases: input.cases.map(entry => ({ id: entry.id, turns: entry.turns.map(({uri: _uri, ...audio}) => audio) })) };
  return { ...input, cases: input.cases.map(entry => ({...entry, turns: entry.turns.map(turn => ({...turn}))})), revision: sha256(new TextEncoder().encode(JSON.stringify(manifest))) };
}
export function datasetMetadata(dataset: BenchmarkDataset) {
  return { dataset_id: dataset.dataset_id, dataset_revision: dataset.revision, dataset_version: dataset.version, dataset_source: dataset.source,
    cases: dataset.cases.map(entry => ({case_id: entry.id, turns: entry.turns.map(turn => ({turn_key: turn.id, sha256: turn.sha256, bytes: turn.bytes, duration_ms: turn.duration_ms, sample_rate: turn.sample_rate, channels: turn.channels}))})) };
}
export function acceptanceFilename(value: string): string {
  if (!value || value.length > 180 || value.startsWith('/') || value.includes('\\') || value.split('/').some(part => !part || part === '.' || part === '..' || !/^[A-Za-z0-9_.-]+$/.test(part))) throw new Error('Acceptance files must use safe relative filenames.');
  return value;
}
