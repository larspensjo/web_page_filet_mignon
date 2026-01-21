#[cfg(target_os = "windows")]
mod platform;

#[cfg(target_os = "windows")]
fn main() -> commanductui::PlatformResult<()> {
    platform::run_app()
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("harvester_app UI is only available on Windows.");
}
