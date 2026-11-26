Pod::Spec.new do |s|
  s.name           = 'TokulyTrim'
  s.version        = '0.1.0'
  s.summary        = 'Cuts a video file down to a time range (AVAssetExportSession).'
  s.description    = 'Cuts a video file down to a time range for Tokuly stories and shorts.'
  s.license        = 'MIT'
  s.author         = 'Tokuly'
  s.homepage       = 'https://github.com/fa0311/better-tokuly'
  s.platforms      = { :ios => '16.4' }
  s.swift_version  = '5.9'
  s.source         = { git: 'https://github.com/fa0311/better-tokuly.git' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  s.source_files = "**/*.{h,m,swift}"
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'SWIFT_COMPILATION_MODE' => 'wholemodule'
  }
end
