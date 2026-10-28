Pod::Spec.new do |s|
  s.name           = 'PhemeNative'
  s.version        = '0.1.0'
  s.summary        = 'On-device Pheme transcription for the Agentic Lab Expo app'
  s.description   = 'Local Whisper inference through the shared Pheme VA Rust/Swift bridge.'
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
  s.source_files = '*.swift', 'Generated/*.swift'
  s.vendored_frameworks = 'Frameworks/PhemeVA.xcframework'
  s.frameworks = 'AVFoundation', 'Accelerate', 'Security', 'SystemConfiguration', 'CryptoKit'
  s.libraries = 'c++', 'resolv'
  unless system('node', File.expand_path('../../../scripts/prepare-native.mjs', __dir__), '--check')
    raise 'Pheme is not prepared. From ios-app run npm run native:prepare after building the production XCFramework.'
  end
end
