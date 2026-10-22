#!/usr/bin/env ruby
# Check Xcode's resolved references, not just the paths supplied to XcodeGen.
require "json"

abort "Usage: check-ios-project.rb PROJECT.xcodeproj FRAMEWORK" unless ARGV.length == 2
project, framework = ARGV.map { |path| File.expand_path(path) }
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
end
puts "PASS: generated framework, target linkage, Swift sources, Info.plist and asset paths"
