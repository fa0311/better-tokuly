Pod::Spec.new do |s|
  s.name           = 'TokulyWebData'
  s.version        = '0.1.0'
  s.summary        = 'Removes what web views stored (WKWebsiteDataStore).'
  s.description    = 'Removes cookies, storage and caches of the web views, for Tokuly web screens.'
  s.license        = 'MIT'
  s.author         = 'Tokuly'
  s.homepage       = 'https://github.com/fa0311/better-tokuly'
  s.platforms      = { :ios => '16.4' }
  s.swift_version  = '5.9'
  s.source         = { git: 'https://github.com/fa0311/better-tokuly.git' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'
  s.frameworks = 'WebKit'

  s.source_files = "**/*.{h,m,swift}"
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'SWIFT_COMPILATION_MODE' => 'wholemodule'
  }
end
