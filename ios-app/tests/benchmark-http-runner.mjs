import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

// This runner never uses API_DATABASE_PATH, API_ADDR or a collector URL inherited
// from a developer shell. Its only writable database is inside its own temp dir.
const root = fileURLToPath(new URL('../../', import.meta.url));
const port = 18081;
const origin = `http://127.0.0.1:${port}`;
const temporary = await mkdtemp(join(tmpdir(), 'pheme-benchmark-http-fixture-'));
const binary = process.env.PHEME_BENCHMARK_TEST_API_BINARY
  ? resolve(process.env.PHEME_BENCHMARK_TEST_API_BINARY) : join(temporary, 'fixture-api');
let api;
let apiExit;
let apiOutput = '';

async function run(command, args, options = {}) {
  const child = spawn(command, args, { cwd: root, stdio: 'inherit', ...options });
  const [code, signal] = await once(child, 'exit');
  if (code !== 0) throw new Error(`${command} exited with ${signal ?? code}`);
}

try {
  // Refuse an occupied port before starting any HTTP requests. In particular,
  // do not silently reuse another API even if its observation table is empty.
  const reservation = createServer();
  reservation.listen(port, '127.0.0.1');
  await once(reservation, 'listening');
  await new Promise((resolveClose, reject) => reservation.close(error => error ? reject(error) : resolveClose()));
  if (!process.env.PHEME_BENCHMARK_TEST_API_BINARY) {
    await run('go', ['build', '-o', binary, './cmd/server'], { cwd: join(root, 'api') });
  }
  api = spawn(binary, [], {
    cwd: temporary, stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, GIN_MODE: 'release', API_ADDR: `127.0.0.1:${port}`, API_DATABASE_PATH: join(temporary, 'fixture.sqlite'), PHEME_VA_URL: '' },
  });
  await once(api, 'spawn');
  apiExit = once(api, 'exit');
  for (const stream of [api.stdout, api.stderr]) stream.on('data', chunk => { apiOutput = (apiOutput + chunk).slice(-16_384); });
  let ready = false;
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (api.exitCode !== null || api.signalCode !== null) throw new Error(`Disposable API exited before readiness.\n${apiOutput}`);
    try {
      const response = await fetch(`${origin}/health`, { redirect: 'error', signal: AbortSignal.timeout(500) });
      if (response.ok) { ready = true; break; }
    } catch { /* The child has not bound its socket yet. */ }
    await delay(50);
  }
  if (!ready) throw new Error(`Disposable API did not become ready.\n${apiOutput}`);
  console.log('Running synthetic phone observation fixtures against an isolated temporary database; no inference or device measurements.');
  await run(process.execPath, ['--test', 'ios-app/tests/benchmark-http.test.ts'], {
    env: { ...process.env, PHEME_BENCHMARK_TEST_URL: origin },
  });
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  if (apiOutput) console.error(apiOutput);
  process.exitCode = 1;
} finally {
  if (api && apiExit && api.exitCode === null && api.signalCode === null) {
    api.kill('SIGTERM');
    const forceStop = setTimeout(() => api.kill('SIGKILL'), 5_000);
    try { await apiExit; } finally { clearTimeout(forceStop); }
  }
  await rm(temporary, { recursive: true, force: true });
}
