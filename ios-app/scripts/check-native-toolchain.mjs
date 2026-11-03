import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const requirement = 'Expo SDK 57 requires full Xcode 26.4+ with Swift 6.3+ and the iOS simulator SDK. Select a supported Xcode using xcode-select or DEVELOPER_DIR.';

function requireVersion(output, pattern, minimum, name) {
  const match = output.match(pattern);
  if (!match) throw new Error(`Cannot determine ${name} version. ${requirement}`);
  const major = Number(match[1]);
  const minor = Number(match[2] ?? 0);
  if (major < minimum[0] || (major === minimum[0] && minor < minimum[1])) {
    throw new Error(`Selected ${name} ${major}.${minor} is too old. ${requirement}`);
  }
}

export function checkNativeToolchain(run = (command, args) =>
  execFileSync(command, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] })) {
  const invoke = (command, args) => {
    try { return run(command, args).trim(); }
    catch { throw new Error(`Cannot run ${command} ${args.join(' ')}. ${requirement}`); }
  };
  const xcode = invoke('xcodebuild', ['-version']);
  requireVersion(xcode, /^Xcode (\d+)(?:\.(\d+))?/m, [26, 4], 'Xcode');
  // xcrun respects the Xcode selected for the native build, unlike a PATH swift.
  const swift = invoke('xcrun', ['swift', '--version']);
  requireVersion(swift, /Apple Swift version (\d+)\.(\d+)/, [6, 3], 'Swift');
  const simulator = invoke('xcrun', ['--sdk', 'iphonesimulator', '--show-sdk-version']);
  if (!/^\d+(?:\.\d+)*$/.test(simulator)) {
    throw new Error(`Cannot determine iOS simulator SDK version. ${requirement}`);
  }
  return `${xcode}\n${swift}\niOS simulator SDK ${simulator}`;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { console.log(checkNativeToolchain()); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
