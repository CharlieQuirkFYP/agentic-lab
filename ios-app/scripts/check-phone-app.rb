#!/usr/bin/env ruby
# Inspect the built product, not only the generated project or source weights.
require "digest"
require "json"
require "open3"

abort "Usage: check-phone-app.rb APP [BUNDLE_ID]" unless (1..2).cover?(ARGV.length)
app = File.expand_path(ARGV.fetch(0))
abort "Missing app bundle: #{app}" unless File.directory?(app)

def plist(path)
  json, error, status = Open3.capture3("plutil", "-convert", "json", "-o", "-", path)
  abort "Cannot read #{path}: #{error}" unless status.success?
  JSON.parse(json)
end

info = plist(File.join(app, "Info.plist"))
bundle_id = info.fetch("CFBundleIdentifier")
abort "Unexpected bundle identifier: #{bundle_id}" if ARGV[1] && bundle_id != ARGV[1]
executable = File.join(app, info.fetch("CFBundleExecutable"))
abort "Missing app executable" unless File.file?(executable)
abort "Missing microphone permission" if info.fetch("NSMicrophoneUsageDescription", "").empty?
config = JSON.parse(File.read(File.expand_path("../app.json", __dir__))).fetch("expo")
abort "Unexpected app version" unless info["CFBundleShortVersionString"] == config.fetch("version")
abort "Unexpected app build number" unless info["CFBundleVersion"] == config.fetch("ios").fetch("buildNumber")
permission = info.fetch("NSLocalNetworkUsageDescription", "")
abort "Missing metrics Local Network permission" unless permission.include?("metrics")
abort "Development-only Local Network permission" if permission.include?("Expo Dev Launcher")
ats = info.fetch("NSAppTransportSecurity", {})
abort "Missing local networking allowance" unless ats["NSAllowsLocalNetworking"] == true
abort "Unexpected unrestricted network allowance" unless ats["NSAllowsArbitraryLoads"] == false
abort "Missing embedded Expo JavaScript (validate a Release build)" unless File.file?(File.join(app, "main.jsbundle"))
scenes = info.dig("UIApplicationSceneManifest", "UISceneConfigurations", "UIWindowSceneSessionRoleApplication")
unless scenes&.any? { |scene| scene["UISceneDelegateClassName"] == "EXExpoAppSceneDelegate" }
  abort "Missing Expo scene lifecycle (required by iOS 27)"
end

framework = File.join(app, "Frameworks", "PhemeReasoning.framework")
framework_info = plist(File.join(framework, "Info.plist"))
reasoning = File.join(framework, framework_info.fetch("CFBundleExecutable"))
abort "Missing embedded reasoning executable" unless File.file?(reasoning)

[executable, reasoning].each do |binary|
  listing, error, status = Open3.capture3("xcrun", "otool", "-L", binary)
  abort "Cannot inspect native dependencies: #{error}" unless status.success?
  dependencies = listing.lines.drop(1).map { |line| line.strip.split(" (").first }
  dependencies.each do |dependency|
    next if dependency.start_with?("/System/Library/", "/usr/lib/")
    next if dependency == "@rpath/PhemeReasoning.framework/PhemeReasoning"
    if binary == executable && dependency.start_with?("@rpath/")
      embedded = File.expand_path(dependency.delete_prefix("@rpath/"), File.join(app, "Frameworks"))
      next if embedded.start_with?(File.join(app, "Frameworks") + "/") && File.file?(embedded)
    end
    abort "Non-portable native dependency in #{binary}: #{dependency}"
  end
  if binary == executable && !dependencies.include?("@rpath/PhemeReasoning.framework/PhemeReasoning")
    abort "App does not load the reasoning framework"
  end
end

exports, error, status = Open3.capture3("xcrun", "nm", "-gU", reasoning)
abort "Cannot inspect reasoning exports: #{error}" unless status.success?
abort "Missing reasoning C ABI" unless exports.match?(/_pheme_reply_create$/)
abort "Reasoning framework leaks GGML/llama symbols" if exports.match?(/ (_ggml_|_gguf_|_llama_)/)

model_root = File.join(app, "PhemeModels.bundle")
# Use the same manifest/selection/license validation as native:prepare; it checks
# every selected model and rejects unexpected extra artifacts in the built app.
verified = system("node", File.expand_path("model-bundle.mjs", __dir__), "--check-resource", model_root)
abort "Bundled model resources failed verification" unless verified
puts "PASS: #{bundle_id} #{info["CFBundleShortVersionString"]} (#{info["CFBundleVersion"]}), Expo JS/scenes, microphone/local-network permissions, embedded reasoning, portable linkage, models and incident role"
