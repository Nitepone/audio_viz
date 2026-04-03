// AudioViz-Bridging-Header.h
// Exposes the Rust audio_viz C FFI to Swift.
//
// This file is set as SWIFT_OBJC_BRIDGING_HEADER in project.yml.
// All aviz_* symbols declared here become directly callable from Swift
// without any import statement.

#include "../bridge/audio_viz.h"
