use core::panic;
use std::cell::RefCell;
use std::sync::{mpsc::Sender, Mutex};
use std::sync::{LazyLock, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use ashpd::WindowIdentifier;
use uiohook_sys::{
    _event_type_EVENT_KEY_PRESSED, _uiohook_event, hook_run, hook_set_dispatch_proc,
    UIOHOOK_SUCCESS,
};
use xcb::Extension;
use xkbcommon::xkb::{self, State};

use crate::errors::{Result, VenbindError};
use crate::structs::{Keybind, KeybindId, KeybindTrigger, Keybinds};
use crate::utils;

static KEYBINDS: LazyLock<Mutex<Keybinds>> = LazyLock::new(|| Mutex::new(Keybinds::default()));
static TX: OnceLock<Sender<KeybindTrigger>> = OnceLock::new();

static IS_USING_PORTAL: AtomicBool = AtomicBool::new(true);

thread_local! {
    static XKBCOMMON_STATE: RefCell<Option<State>> = RefCell::new(None);
}

// window_id should be either a XID if using an X server or a wayland surface handle
// display_id should be a wayland display handle, or None if using X
pub(crate) fn start_keybinds_internal(
    window_id: Option<usize>,
    display_id: Option<usize>,
    tx: Sender<KeybindTrigger>,
) -> Result<()> {
    TX.set(tx).unwrap();
    let result = xdg_start_keybinds(window_id, display_id);
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("Failed to start using XDG Desktop Portals: {}", e);
            IS_USING_PORTAL.store(false, Ordering::Relaxed);
            xcb_start_keybinds()
        }
    }
}

pub(crate) fn register_keybind_internal(keybind: String, id: KeybindId) -> Result<()> {
    if IS_USING_PORTAL.load(Ordering::Relaxed) {
        Err(VenbindError::Message("todo".to_owned()))
    } else {
        xcb_register_keybind(keybind, id)
    }
}

pub(crate) fn unregister_keybind_internal(id: KeybindId) -> Result<()> {
    if IS_USING_PORTAL.load(Ordering::Relaxed) {
        Err(VenbindError::Message("todo".to_owned()))
    } else {
        xcb_unregister_keybind(id)
    }
}

#[no_mangle]
pub unsafe extern "C" fn xdg_displatch_proc(event_ref: *mut _uiohook_event) {

}

#[inline]
fn convert_to_pointer<T>(value: Option<usize>) -> *mut T {
    assert_eq!(size_of::<usize>(), size_of::<*mut T>());
    match value {
        Some(value) => value as *mut T,
        None => std::ptr::null_mut(),
    }
}

fn xdg_start_keybinds(
    window_id: Option<usize>,
    display_id: Option<usize>,
) -> Result<()> {
    use ashpd::desktop::global_shortcuts::*;

    if window_id.is_none() {
        return Err(VenbindError::Message("Window ID is Not Valid".to_owned()));
    }

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => return Err(VenbindError::Message("Failed to create tokio runtime".to_owned())),
    };

    let res = runtime.block_on(async {
        let portal = GlobalShortcuts::new().await?;

        let session = portal.create_session().await?;

        let window_handle = if utils::is_wayland() {
            if display_id.is_none() {
                return Err(VenbindError::Message("Wayland requires a valid display handle".to_owned()));
            }
            unsafe {
                WindowIdentifier::from_wayland_raw(convert_to_pointer(window_id), convert_to_pointer(display_id)).await
            }
        } else {
            WindowIdentifier::from_xid(window_id.unwrap() as _)
        };

        Ok((portal, session, window_handle))
    });

    match res {
        Ok((portal, session, window_handle)) => { Ok(()) },
        Err(e) => Err(e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn xcb_dispatch_proc(event_ref: *mut _uiohook_event) {
    let event = *event_ref;
    if event.type_ == _event_type_EVENT_KEY_PRESSED {
        XKBCOMMON_STATE.with(|state| {
            if let Some(state) = &*state.borrow() {
                let keycode =
                    uiohook_sys::platform::scancode_to_keycode(event.data.keyboard.keycode);
                let key = state.key_get_utf8(keycode.into());
                let shift = event.mask & uiohook_sys::MASK_SHIFT as u16 != 0;
                let alt = event.mask & uiohook_sys::MASK_ALT as u16 != 0;
                let ctrl = event.mask & uiohook_sys::MASK_CTRL as u16 != 0;
                let keybind = Keybind {
                    shift,
                    alt,
                    ctrl,
                    character: if !key.is_empty() { Some(key) } else { None },
                };
                let keybinds = KEYBINDS.lock();
                if let Some(id) = keybinds.unwrap().get_keybind_id(keybind) {
                    TX.get().unwrap().send(KeybindTrigger::Pressed(id)).unwrap();
                }
            } else {
                panic!("The state is gone???? how????");
            }
        });
    }
}

fn xcb_start_keybinds() -> Result<()> {
    let (connection, _screen) =
        xcb::Connection::connect_with_extensions(None, &[Extension::Xkb], &[]).unwrap();
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::x11::setup_xkb_extension(
        &connection,
        xkb::x11::MIN_MAJOR_XKB_VERSION,
        xkb::x11::MIN_MINOR_XKB_VERSION,
        xkb::x11::SetupXkbExtensionFlags::NoFlags,
        &mut 0,
        &mut 0,
        &mut 0,
        &mut 0,
    );
    let device_id = xkb::x11::get_core_keyboard_device_id(&connection);
    let keymap = xkb::x11::keymap_new_from_device(
        &context,
        &connection,
        device_id,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    );
    drop(connection);
    // don't make a state with an xcb connection (state_new_from_device) so it only chooses the first layout
    // TODO: if someone's first selected layout is not a latin based layout horrible things happen
    let state = xkb::State::new(&keymap);
    XKBCOMMON_STATE.replace(Some(state));
    unsafe {
        hook_set_dispatch_proc(Some(xcb_dispatch_proc));
        if hook_run() != UIOHOOK_SUCCESS as i32 {
            return Err(VenbindError::LibUIOHookError);
        }
    };
    Ok(())
}

fn xcb_register_keybind(keybind: String, id: KeybindId) -> Result<()> {
    let keybind = Keybind::from_string(keybind);
    let mut keybinds = KEYBINDS.lock().unwrap();
    keybinds.register_keybind(keybind, id);
    Ok(())
}

fn xcb_unregister_keybind(id: KeybindId) -> Result<()> {
    let mut keybinds = KEYBINDS.lock().unwrap();
    keybinds.unregister_keybind(id);
    Ok(())
}
