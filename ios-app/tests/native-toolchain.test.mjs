import assert from 'node:assert/strict';
import test from 'node:test';
import { checkNativeToolchain } from '../scripts/check-native-toolchain.mjs';

function runner(xcode = '26.4', swift = '6.3', simulator = '26.4') {
  return (command, args) => {
    if (command === 'xcodebuild') {
      assert.deepEqual(args, ['-version']);
      return `Xcode ${xcode}\nBuild version example`;
    }
    assert.equal(command, 'xcrun');
    if (args[0] === 'swift') {
      assert.deepEqual(args, ['swift', '--version']);
      return `swift-driver version: 1.0 Apple Swift version ${swift} (swiftlang-example)`;
    }
    assert.deepEqual(args, ['--sdk', 'iphonesimulator', '--show-sdk-version']);
    return simulator;
  };
}

test('rejects the Xcode 26.0.1 toolchain from the failed native-link run', () => {
  assert.throws(() => checkNativeToolchain(runner('26.0.1', '6.2')),
    /Xcode 26\.0 is too old.*Xcode 26\.4\+ with Swift 6\.3\+/);
});

test('checks the selected Swift compiler even with a supported Xcode', () => {
  assert.throws(() => checkNativeToolchain(runner('26.4', '6.2.3')), /Swift 6\.2 is too old/);
});

test('accepts the minimum and newer Xcode toolchains', () => {
  assert.match(checkNativeToolchain(runner()), /iOS simulator SDK 26\.4/);
  assert.match(checkNativeToolchain(runner('27.2', '6.4.2', '27.2')), /Xcode 27\.2/);
});

test('rejects unavailable or unrecognizable native tools and SDKs', () => {
  assert.throws(() => checkNativeToolchain(() => { throw new Error('Command Line Tools only'); }),
    /Cannot run xcodebuild -version.*full Xcode/);
  assert.throws(() => checkNativeToolchain(() => 'unknown'), /Cannot determine Xcode version/);
  assert.throws(() => checkNativeToolchain(runner('26.4', 'unknown')), /Cannot determine Swift version/);
  assert.throws(() => checkNativeToolchain(runner('26.4', '6.3', 'unknown')), /Cannot determine iOS simulator SDK/);
  assert.throws(() => checkNativeToolchain((command, args) => {
    if (args[0] === '--sdk') throw new Error('SDK missing');
    return runner()(command, args);
  }), /Cannot run xcrun --sdk iphonesimulator/);
});
