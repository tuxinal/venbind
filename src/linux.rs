use core::panic;
use std::cell::RefCell;
use std::sync::{mpsc::Sender, Mutex};
use std::sync::{LazyLock, OnceLock};

use uiohook_sys::{
    _event_type_EVENT_KEY_PRESSED, _uiohook_event, hook_run, hook_set_dispatch_proc,
    UIOHOOK_SUCCESS,
};
use xcb::Extension;
use xkbcommon::xkb::{self, State};

use crate::errors::{Result, VenkeybindError};
use crate::structs::{Keybind, KeybindId, KeybindTrigger, Keybinds};
use crate::utils;

static KEYBINDS: LazyLock<Mutex<Keybinds>> = LazyLock::new(|| Mutex::new(Keybinds::default()));
static TX: OnceLock<Sender<KeybindTrigger>> = OnceLock::new();

thread_local! {
    static XKBCOMMON_STATE: RefCell<Option<State>> = RefCell::new(None);
}

pub(crate) fn start_keybinds_internal(
    window_id: Option<u64>,
    tx: Sender<KeybindTrigger>,
) -> Result<()> {
    TX.set(tx).unwrap();
    if utils::is_wayland() {
        return Err(VenkeybindError::Message("todo".to_owned()));
    }
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
        hook_set_dispatch_proc(Some(dispatch_proc));
        if hook_run() != UIOHOOK_SUCCESS as i32 {
            return Err(VenkeybindError::LibUIOHookError);
        }
    };
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn dispatch_proc(event_ref: *mut _uiohook_event) {
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

pub(crate) fn register_keybind_internal(keybind: String, id: KeybindId) -> Result<()> {
    if utils::is_wayland() {
        return Err(VenkeybindError::Message("todo".to_owned()));
    }
    let keybind = Keybind::from_string(keybind);
    let mut keybinds = KEYBINDS.lock().unwrap();
    keybinds.register_keybind(keybind, id);
    Ok(())
}
pub(crate) fn unregister_keybind_internal(id: KeybindId) -> Result<()> {
    if utils::is_wayland() {
        return Err(VenkeybindError::Message("todo".to_owned()));
    }
    let mut keybinds = KEYBINDS.lock().unwrap();
    keybinds.unregister_keybind(id);
    Ok(())
}
