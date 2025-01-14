use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::{mpsc::Sender, Mutex};
use std::sync::{LazyLock, OnceLock};

use uiohook_sys::{
    _event_type_EVENT_KEY_PRESSED, _uiohook_event, hook_run, hook_set_dispatch_proc,
    UIOHOOK_SUCCESS,
};

use crate::errors::{Result, VenbindError};
use crate::structs::{Keybind, KeybindId, KeybindTrigger, Keybinds};

static KEYBINDS: LazyLock<Mutex<Keybinds>> = LazyLock::new(|| Mutex::new(Keybinds::default()));
static TX: OnceLock<Sender<KeybindTrigger>> = OnceLock::new();

pub(crate) fn start_keybinds_internal(tx: Sender<KeybindTrigger>) -> Result<()> {
    TX.set(tx).unwrap();

    // don't make a state with an xcb connection (state_new_from_device) so it only chooses the first layout
    // TODO: if someone's first selected layout is not a latin based layout horrible things happen
    unsafe {
        hook_set_dispatch_proc(Some(dispatch_proc));
        if hook_run() != UIOHOOK_SUCCESS as i32 {
            return Err(VenbindError::LibUIOHookError);
        }
    };
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn dispatch_proc(event_ref: *mut _uiohook_event) {
    let event = *event_ref;
    if event.type_ == _event_type_EVENT_KEY_PRESSED {
        let keycode = uiohook_sys::platform::scancode_to_keycode(event.data.keyboard.keycode);

        const BUF_SIZE: usize = 8;
        let mut key_buffer: Vec<uiohook_sys::platform::wchar_t> = vec![0; BUF_SIZE];
        let str_count = uiohook_sys::platform::keycode_to_unicode(
            keycode,
            key_buffer.as_mut_ptr(),
            BUF_SIZE.try_into().unwrap(),
        );

        key_buffer.truncate(str_count.try_into().unwrap());
        let key = OsString::from_wide(&key_buffer);
        let shift = event.mask & uiohook_sys::MASK_SHIFT as u16 != 0;
        let alt = event.mask & uiohook_sys::MASK_ALT as u16 != 0;
        let ctrl = event.mask & uiohook_sys::MASK_CTRL as u16 != 0;
        let keybind = Keybind {
            shift,
            alt,
            ctrl,
            character: if !key.is_empty() {
                Some(key.to_string_lossy().to_lowercase())
            } else {
                None
            },
        };

        let keybinds = KEYBINDS.lock();
        if let Some(id) = keybinds.unwrap().get_keybind_id(keybind) {
            TX.get().unwrap().send(KeybindTrigger::Pressed(id)).unwrap();
        }
    }
}

pub(crate) fn register_keybind_internal(keybind: String, id: KeybindId) -> Result<()> {
    let keybind = Keybind::from_string(keybind);
    let mut keybinds = KEYBINDS.lock().unwrap();
    keybinds.register_keybind(keybind, id);
    Ok(())
}
pub(crate) fn unregister_keybind_internal(id: KeybindId) -> Result<()> {
    let mut keybinds = KEYBINDS.lock().unwrap();
    keybinds.unregister_keybind(id);
    Ok(())
}
