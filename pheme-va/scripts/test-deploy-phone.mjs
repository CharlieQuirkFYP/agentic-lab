// Offline deployment and final-bundle checks. No Xcode, signing, or weights.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync, copyFileSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const source = path.dirname(fileURLToPath(import.meta.url));
const temp = mkdtempSync(path.join(tmpdir(), "pheme-deploy-"));
const root = path.join(temp, "workspace with spaces");
const bin = path.join(temp, "bin");
const log = path.join(temp, "commands.log");
const hash = (text) => createHash("sha256").update(text).digest("hex");
const put = (name, text, mode) => {
  mkdirSync(path.dirname(name), { recursive: true });
  writeFileSync(name, text, { mode });
};
for (const file of ["deploy-phone.sh", "check-phone-app.rb"]) {
  mkdirSync(path.join(root, "scripts"), { recursive: true });
  copyFileSync(path.join(source, file), path.join(root, "scripts", file));
}
put(path.join(root, "roles/incident-reporting.txt"), "Test incident role\n");
put(path.join(root, "models/manifest.toml"), `
[[models]]
id = "whisper-large-v3-turbo"
model = "transcript/whisper/ggml-large-v3-turbo.bin"
sha256 = "${hash("asr")}"
model_size_bytes = 3
[[models]]
id = "qwen2.5-1.5b-instruct-q4-k-m"
model = "reply/qwen2.5-1.5b-instruct-q4_k_m.gguf"
sha256 = "${hash("reply")}"
model_size_bytes = 5
`);
for (const name of ["build-phone.sh"]) {
  put(path.join(root, "scripts", name), `#!/usr/bin/env bash
set -euo pipefail
printf '%s %s app_models=%s fixture_test=%s\\n' "\${0##*/}" "$*" "\${PHEME_IOS_APP_MODELS:-}" "\${PHEME_IOS_MODEL_TEST:-}" >> "$FAKE_LOG"
exit "\${FAKE_PREPARE_STATUS:-0}"
`, 0o755);
}
put(path.join(root, "scripts/check-ios-project.rb"), `
File.open(ENV.fetch("FAKE_LOG"), "a") { |file| file.puts("project-preflight " + ARGV.join(" ")) }
exit Integer(ENV.fetch("FAKE_PROJECT_STATUS", "0"))
`);
for (const framework of ["whisper/PhemeVA.xcframework", "reply/PhemeReasoning.xcframework"]) {
  mkdirSync(path.join(root, "target/ios", framework), { recursive: true });
}
put(path.join(root, "hosts/ios/ExportOptions.plist"), "test export options");
mkdirSync(bin, { recursive: true });
const fake = path.join(bin, "fake-tool");
put(fake, `#!/usr/bin/env ruby
require "json"
require "fileutils"
name = File.basename($0)
File.open(ENV.fetch("FAKE_LOG"), "a") { |f| f.puts(([name] + ARGV).join(" ")) }
case name
when "xcodegen"
  exit Integer(ENV.fetch("FAKE_GENERATE_STATUS", "0")) unless ENV.fetch("FAKE_GENERATE_STATUS", "0") == "0"
  root = ARGV[ARGV.index("--project") + 1]
  FileUtils.mkdir_p(File.join(root, "PhemePhone.xcodeproj"))
  File.open(ENV.fetch("FAKE_LOG"), "a") { |f| f.puts("swift_flags=" + ENV.fetch("PHEME_SWIFT_FLAGS")) }
when "git"
  puts "test-head"
when "codesign"
  exit Integer(ENV.fetch("FAKE_SIGN_STATUS", "0"))
when "unzip"
  exit Integer(ENV.fetch("FAKE_IPA_STATUS", "0"))
when "plutil"
  puts File.read(ARGV.last)
when "xcrun"
  if ARGV[0, 2] == ["simctl", "list"]
    phones = ENV["FAKE_NO_SIMULATOR"] == "1" ? [] : [{"name"=>"iPhone Test", "udid"=>"sim-1", "isAvailable"=>true, "state"=>"Booted"}]
    puts JSON.generate({"devices"=>{"runtime"=>phones}})
  elsif ARGV[0] == "otool"
    puts ARGV.last + ":"
    dependency = ENV.fetch("FAKE_DEPENDENCY", "@rpath/PhemeReasoning.framework/PhemeReasoning")
    puts "\\t" + dependency + " (compatibility version 0.0.0, current version 0.0.0)"
    puts "\\t/usr/lib/libc++.1.dylib (compatibility version 1.0.0, current version 1.0.0)"
  elsif ARGV[0] == "nm"
    puts "00000000 T _pheme_reply_create"
    puts "00000000 T _ggml_leak" if ENV["FAKE_SYMBOL_LEAK"] == "1"
  elsif ARGV.include?("install") && ENV["FAKE_INSTALL_STATUS"]
    exit Integer(ENV["FAKE_INSTALL_STATUS"])
  end
when "xcodebuild"
  if ARGV[0] == "-version"
    puts "Xcode fake"
  elsif ARGV[0] == "-exportArchive"
    exit Integer(ENV.fetch("FAKE_EXPORT_STATUS", "0")) unless ENV.fetch("FAKE_EXPORT_STATUS", "0") == "0"
    out = ARGV[ARGV.index("-exportPath") + 1]
    FileUtils.mkdir_p(out)
    File.write(File.join(out, "PhemePhone.ipa"), "fake ipa")
  else
    exit Integer(ENV.fetch("FAKE_BUILD_STATUS", "0")) unless ENV.fetch("FAKE_BUILD_STATUS", "0") == "0"
    if ARGV[0] == "archive"
      app = File.join(ARGV[ARGV.index("-archivePath") + 1], "Products/Applications/PhemePhone.app")
    else
      platform = ARGV.join(" ").include?("iOS Simulator") ? "iphonesimulator" : "iphoneos"
      app = File.join(ARGV[ARGV.index("-derivedDataPath") + 1], "Build/Products/Release-" + platform + "/PhemePhone.app")
    end
    framework = File.join(app, "Frameworks/PhemeReasoning.framework")
    FileUtils.mkdir_p(framework)
    identifier = ARGV.find { |a| a.start_with?("PRODUCT_BUNDLE_IDENTIFIER=") }.split("=", 2).last
    File.write(File.join(app, "Info.plist"), JSON.generate({"CFBundleIdentifier"=>identifier, "CFBundleExecutable"=>"PhemePhone", "NSMicrophoneUsageDescription"=>"Test microphone"}))
    File.write(File.join(app, "PhemePhone"), "fake app")
    File.write(File.join(framework, "Info.plist"), JSON.generate({"CFBundleExecutable"=>"PhemeReasoning"}))
    File.write(File.join(framework, "PhemeReasoning"), "fake framework")
    models = File.join(app, "PhemeModels")
    FileUtils.mkdir_p(File.join(models, "ASR"))
    FileUtils.mkdir_p(File.join(models, "Reasoning"))
    File.write(File.join(models, "ASR/ggml-large-v3-turbo.bin"), ENV["FAKE_CORRUPT_MODEL"] == "1" ? "bad" : "asr")
    File.write(File.join(models, "Reasoning/qwen2.5-1.5b-instruct-q4_k_m.gguf"), "reply")
    File.write(File.join(models, "incident-reporting.txt"), "Test incident role\\n")
    File.write(File.join(models, "MODEL-NOTICES.txt"), "Test notices")
  end
end
`, 0o755);
for (const tool of ["git", "codesign", "plutil", "xcrun", "xcodebuild", "xcodegen", "unzip"]) symlinkSync(fake, path.join(bin, tool));

// Apple's rbconfig consults xcrun when SDKROOT is unset.
const cleanEnv = { ...process.env, PATH: `${bin}:${process.env.PATH}`, FAKE_LOG: log,
  RUBYOPT: "--disable-gems", SDKROOT: "/fake/SDK" };
for (const key of Object.keys(cleanEnv)) if (key.startsWith("PHEME_IOS_")) delete cleanEnv[key];
const run = (args, extra = {}, status = 0) => {
  writeFileSync(log, "");
  const result = spawnSync("bash", [path.join(root, "scripts/deploy-phone.sh"), ...args], {
    env: { ...cleanEnv, ...extra }, encoding: "utf8", timeout: 30_000,
  });
  assert.equal(result.status, status, result.stdout + result.stderr);
  return readFileSync(log, "utf8");
};
try {
  assert.equal(run(["bad"], {}, 2), "");
  assert.equal(run(["simulator", "--unknown"], {}, 2), "");
  assert.equal(run(["device"], {}, 2), "");
  assert.equal(run(["device"], { PHEME_IOS_DEVELOPMENT_TEAM: "TEAM" }, 2), "");
  assert.equal(run(["simulator"], { PHEME_IOS_BUNDLE_ID: "not an identifier" }, 2), "");
  let commands = run(["simulator", "--use-prepared"]);
  assert.match(commands, /xcodegen generate .*phone-simulator\/project/);
  assert.match(commands, /swift_flags=PHEME_WHISPER PHEME_REASONING/);
  assert.match(commands, /project-preflight/);
  assert.doesNotMatch(commands, /test-ios.sh|whisper\/project\/PhemePhone/);
  assert.match(commands, /CODE_SIGNING_ALLOWED=NO/);
  assert.match(commands, /simctl install sim-1 .*PhemePhone.app/);
  assert.match(commands, /simctl launch sim-1 org.agenticlab.PhemePhone/);
  assert.doesNotMatch(commands, /codesign|devicectl/);
  commands = run(["simulator", "--build-only"]);
  assert.match(commands, /build-phone.sh  app_models=1 fixture_test=0/);
  assert.doesNotMatch(commands, /simctl (install|launch)/);
  const deviceEnv = { PHEME_IOS_DEVELOPMENT_TEAM: "TEAM", PHEME_IOS_DEVICE_ID: "device-1", PHEME_IOS_BUNDLE_ID: "org.example.Pheme" };
  commands = run(["device", "--use-prepared"], deviceEnv);
  assert.match(commands, /DEVELOPMENT_TEAM=TEAM/);
  assert.match(commands, /codesign --verify --deep --strict/);
  assert.match(commands, /devicectl device install app --device device-1/);
  assert.match(commands, /devicectl device process launch --device device-1 org.example.Pheme/);
  commands = run(["archive", "--use-prepared"], deviceEnv);
  assert.match(commands, /xcodebuild archive/);
  assert.match(commands, /xcodebuild -exportArchive/);
  assert.match(commands, /unzip -tq .*PhemePhone.ipa/);
  assert.match(readFileSync(path.join(root, "target/ios/deploy/archive/ipa-sha256.txt"), "utf8"), /^[a-f0-9]{64} /);
  assert.doesNotMatch(commands, /devicectl|simctl/);
  for (const [extra, expected] of [
    [{ FAKE_BUILD_STATUS: "42" }, 42],
    [{ FAKE_CORRUPT_MODEL: "1" }, 1],
    [{ FAKE_DEPENDENCY: "/Users/developer/libllama.dylib" }, 1],
    [{ FAKE_SYMBOL_LEAK: "1" }, 1],
  ]) {
    commands = run(["simulator", "--use-prepared"], extra, expected);
    assert.doesNotMatch(commands, /simctl (install|launch)/);
  }
  commands = run(["device", "--use-prepared"], { ...deviceEnv, FAKE_SIGN_STATUS: "43" }, 43);
  assert.doesNotMatch(commands, /devicectl/);
  commands = run(["device", "--use-prepared"], { ...deviceEnv, FAKE_INSTALL_STATUS: "44" }, 44);
  assert.doesNotMatch(commands, /devicectl device process launch/);
  run(["archive", "--use-prepared"], { ...deviceEnv, FAKE_EXPORT_STATUS: "45" }, 45);
  run(["archive", "--use-prepared"], { ...deviceEnv, FAKE_IPA_STATUS: "48" }, 48);
  commands = run(["simulator"], { FAKE_PREPARE_STATUS: "46" }, 46);
  assert.doesNotMatch(commands, /xcodebuild/);
  for (const extra of [{ FAKE_GENERATE_STATUS: "47" }, { FAKE_PROJECT_STATUS: "47" }]) {
    commands = run(["simulator", "--use-prepared"], extra, 47);
    assert.doesNotMatch(commands, /xcodebuild/);
  }
  run(["simulator", "--use-prepared"], { FAKE_NO_SIMULATOR: "1" }, 1);
  console.log("PASS: simulator/device/archive deployment, bundle integrity, signing, portable linkage, failure gates and paths with spaces");
} finally {
  rmSync(temp, { recursive: true, force: true });
}
