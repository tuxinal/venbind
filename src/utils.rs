use std::env;

#[cfg(target_os = "linux")]
pub(crate) fn is_wayland() -> bool {
    env::var("WAYLAND_DISPLAY").is_ok()
}