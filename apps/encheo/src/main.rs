// Windows: release builds are GUI apps, so no console window opens behind Encheo. Debug builds
// keep the console for log output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> Result<(), slint::PlatformError> {
    encheo::run()
}
