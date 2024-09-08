use std::env;

#[cfg(target_os = "linux")]
pub(crate) fn is_wayland() -> bool {
    env::var("WAYLAND_DISPLAY").is_ok()
}

#[cfg(target_os = "linux")]
pub(crate) fn use_xdg_on_x11() -> bool {
    env::var("VENBIND_USE_XDG_PORTAL").is_ok()
}
