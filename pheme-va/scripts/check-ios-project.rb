#!/usr/bin/env ruby
# Check Xcode's resolved references, not just the paths supplied to XcodeGen.
require "json"

abort "Usage: check-ios-project.rb PROJECT.xcodeproj FRAMEWORK [REASONING_FRAMEWORK]" unless [2, 3].include?(ARGV.length)
project, framework, reasoning_framework = ARGV.map { |path| File.expand_path(path) }
root = File.dirname(project)
json = IO.popen(["plutil", "-convert", "json", "-o", "-", "#{project}/project.pbxproj"], &:read)
abort "Could not parse generated Xcode project" unless $?.success?
data = JSON.parse(json)
objects = data.fetch("objects")
resolved = {}

walk = lambda do |id, parent|
  object = objects.fetch(id)
  tree = object.fetch("sourceTree", "<group>")
  base = case tree
         when "<group>" then parent
         when "SOURCE_ROOT" then root
         when "<absolute>" then "/"
         else return # SDK and build-product paths are resolved by Xcode.
         end
  path = File.expand_path(object.fetch("path", ""), base)
  if object["isa"] == "PBXFileReference"
    resolved[id] = path
  else
    object.fetch("children", []).each { |child| walk.call(child, path) }
  end
end
walk.call(objects.fetch(data.fetch("rootObject")).fetch("mainGroup"), root)

framework_ids = resolved.select { |_, path| File.basename(path) == "PhemeVA.xcframework" }.keys
abort "Expected exactly one PhemeVA framework reference" unless framework_ids.length == 1
actual = resolved.fetch(framework_ids.first)
unless actual == framework && File.directory?(actual)
  abort "Incorrect XCFramework reference: #{actual}; expected existing #{framework}"
end

%w[PhemePhone BridgeTests].each do |name|
  target = objects.values.find { |obj| obj["isa"] == "PBXNativeTarget" && obj["name"] == name }
  abort "Missing target #{name}" unless target
  linked = target.fetch("buildPhases").flat_map do |id|
    phase = objects.fetch(id)
    phase["isa"] == "PBXFrameworksBuildPhase" ? phase.fetch("files", []) : []
  end.any? { |id| framework_ids.include?(objects.fetch(id)["fileRef"]) }
  abort "#{name} does not link PhemeVA" unless linked
end

reasoning_ids = resolved.select { |_, path| File.basename(path) == "PhemeReasoning.xcframework" }.keys
if reasoning_framework
  abort "Expected exactly one reasoning framework reference" unless reasoning_ids.length == 1
  actual = resolved.fetch(reasoning_ids.first)
  abort "Incorrect reasoning framework reference: #{actual}" unless actual == reasoning_framework && File.directory?(actual)
  %w[PhemePhone BridgeTests].each do |name|
    target = objects.values.find { |obj| obj["isa"] == "PBXNativeTarget" && obj["name"] == name }
    linked = target.fetch("buildPhases").flat_map do |id|
      phase = objects.fetch(id)
      phase["isa"] == "PBXFrameworksBuildPhase" ? phase.fetch("files", []) : []
    end.any? { |id| reasoning_ids.include?(objects.fetch(id)["fileRef"]) }
    abort "#{name} does not link reasoning" unless linked
  end
  app = objects.values.find { |obj| obj["isa"] == "PBXNativeTarget" && obj["name"] == "PhemePhone" }
  embedded = app.fetch("buildPhases").flat_map do |id|
    phase = objects.fetch(id)
    phase["isa"] == "PBXCopyFilesBuildPhase" ? phase.fetch("files", []) : []
  end.any? { |id| reasoning_ids.include?(objects.fetch(id)["fileRef"]) }
  abort "App does not embed reasoning framework" unless embedded
elsif !reasoning_ids.empty?
  abort "Unexpected reasoning framework in non-bundled project"
end

sources = resolved.values.select { |path| File.extname(path) == ".swift" }
abort "Missing Swift source references" if sources.empty?
sources.each { |path| abort "Missing Swift source: #{path}" unless File.file?(path) }
plists = objects.values.map do |obj|
  obj["buildSettings"] && obj["buildSettings"]["INFOPLIST_FILE"]
end.compact
abort "Missing app Info.plist setting" if plists.empty?
plists.each do |path|
  abort "Missing generated Info.plist: #{path}" unless File.file?(File.expand_path(path, root))
end
resolved.values.select { |path| File.basename(path) == "assets" }.each do |path|
  expected = File.expand_path("../../assets", root)
  abort "Incorrect test assets reference: #{path}" unless path == expected
  abort "Missing test assets directory: #{path}" unless File.directory?(path)
end
model_ids = resolved.select { |_, path| File.basename(path) == "PhemeModels" }.keys
if reasoning_framework
  expected = File.expand_path("../../app-models/PhemeModels", root)
  abort "Missing bundled model resource" unless model_ids.length == 1 && resolved.fetch(model_ids.first) == expected
  %w[ASR/ggml-large-v3-turbo.bin Reasoning/qwen2.5-1.5b-instruct-q4_k_m.gguf incident-reporting.txt].each do |file|
    abort "Missing bundled model asset: #{file}" unless File.file?(File.join(expected, file))
  end
  %w[PhemePhone BridgeTests].each do |name|
    target = objects.values.find { |obj| obj["isa"] == "PBXNativeTarget" && obj["name"] == name }
    count = target.fetch("buildPhases").flat_map do |id|
      phase = objects.fetch(id)
      phase["isa"] == "PBXResourcesBuildPhase" ? phase.fetch("files", []) : []
    end.count { |id| model_ids.include?(objects.fetch(id)["fileRef"]) }
    abort "Incorrect model resource copy count for #{name}" unless count == (name == "PhemePhone" ? 1 : 0)
  end
elsif !model_ids.empty?
  abort "Unexpected bundled models in ordinary project"
end
puts "PASS: generated framework, target linkage, Swift sources, Info.plist and asset paths"
