Pod::Spec.new do |s|
  s.name           = 'PhemeNative'
  s.version        = '0.1.0'
  s.summary        = 'On-device Pheme speech and reasoning for the Expo app'
  s.description   = 'Expo-owned Swift bridges to the Pheme Rust core and isolated reasoning runtime.'
  s.author        = 'Agentic Lab'
  s.homepage      = 'https://github.com/CharlieQuirkFYP/agentic-lab'
  s.license       = { :type => 'MIT' }
  s.platforms     = { :ios => '17.0' }
  s.source        = { git: 'https://github.com/CharlieQuirkFYP/agentic-lab.git' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  # Swift/Objective-C compatibility
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
  }

  s.swift_version = '5.0'
  s.source_files = '*.swift'
  s.vendored_frameworks = 'Frameworks/PhemeVA.xcframework', 'Frameworks/PhemeReasoning.xcframework'
  if File.directory?(File.join(__dir__, 'Resources/PhemeModels'))
    s.resource_bundles = { 'PhemeModels' => ['Resources/PhemeModels/*'] }
  end
  s.frameworks = 'AVFoundation', 'Accelerate', 'Security', 'SystemConfiguration', 'CryptoKit'
  s.libraries = 'c++', 'resolv'
  unless system('node', File.expand_path('../../../scripts/prepare-native.mjs', __dir__), '--check')
    raise 'Pheme is not prepared. From ios-app run npm run native:build, then npm run native:bundle for bundled models.'
  end
end
