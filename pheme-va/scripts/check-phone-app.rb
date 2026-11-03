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

model_root = File.join(app, "PhemeModels")
manifest = File.read(File.expand_path("../models/manifest.toml", __dir__))
blocks = manifest.split(/^\[\[models\]\]\s*$/)
{
  "whisper-large-v3-turbo" => "ASR",
  "qwen2.5-1.5b-instruct-q4-k-m" => "Reasoning"
}.each do |id, directory|
  block = blocks.find { |entry| entry.match?(/^id = "#{Regexp.escape(id)}"$/) }
  abort "Missing model catalog entry: #{id}" unless block
  filename = block.match(/^model = "([^"]+)"$/)[1].split("/").last
  sha256 = block.match(/^sha256 = "([a-f0-9]{64})"$/)[1]
  bytes = Integer(block.match(/^model_size_bytes = (\d+)$/)[1])
  path = File.join(model_root, directory, filename)
  abort "Missing bundled model: #{path}" unless File.file?(path)
  abort "Incorrect bundled model size: #{path}" unless File.size(path) == bytes
  abort "Incorrect bundled model checksum: #{path}" unless Digest::SHA256.file(path).hexdigest == sha256
  puts "PASS: #{id}, #{bytes} bytes, SHA-256 #{sha256}"
end
prompt = File.join(model_root, "incident-reporting.txt")
source_prompt = File.expand_path("../roles/incident-reporting.txt", __dir__)
abort "Missing or stale bundled incident role" unless File.file?(prompt) && File.binread(prompt) == File.binread(source_prompt)
abort "Missing model notices" unless File.file?(File.join(model_root, "MODEL-NOTICES.txt"))
puts "PASS: #{bundle_id}, microphone permission, embedded reasoning, portable linkage, models and incident role"
