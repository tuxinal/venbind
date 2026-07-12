use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::{mpsc::Sender, Mutex};
use std::sync::{LazyLock, OnceLock};

use uiohook_sys::{
    _event_type_EVENT_KEY_PRESSED, _event_type_EVENT_KEY_RELEASED, _uiohook_event, hook_run,
    hook_set_dispatch_proc, UIOHOOK_SUCCESS,
};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F10, VK_F11,
    VK_F12, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_HOME, VK_INSERT,
    VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RCONTROL,
    VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};

use crate::errors::{Result, VenbindError};
use crate::structs::{KeybindId, KeybindInfo, KeybindTrigger, Keybinds, Shortcut};

static KEYBINDS: LazyLock<Mutex<Keybinds>> = LazyLock::new(|| Mutex::new(Keybinds::default()));
static CURR_DOWN: LazyLock<Mutex<Shortcut>> = LazyLock::new(|| {
    Mutex::new(Shortcut {
        shift: false,
        alt: false,
        ctrl: false,
        meta: false,
        keys: HashSet::new(),
    })
});
static CURR_ACTIVE_KEYBINDS: LazyLock<Mutex<HashSet<KeybindId>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static TX: OnceLock<Sender<KeybindTrigger>> = OnceLock::new();

pub(crate) fn start_keybinds_internal(tx: Sender<KeybindTrigger>, _: Option<String>) -> Result<()> {
    TX.set(tx).unwrap();

    unsafe {
        hook_set_dispatch_proc(Some(dispatch_proc));
        if hook_run() != UIOHOOK_SUCCESS as i32 {
            return Err(VenbindError::LibUIOHookError);
        }
    };
    Ok(())
}

#[no_mangle]
pub extern "C" fn dispatch_proc(event_ref: *mut _uiohook_event) {
    let event = unsafe { *event_ref };
    if event.type_ == _event_type_EVENT_KEY_PRESSED || event.type_ == _event_type_EVENT_KEY_RELEASED
    {
        let keycode = unsafe { event.data.keyboard.rawcode };
        let vk = VIRTUAL_KEY(keycode);
        let key: Option<String> = match vk {
            VK_SHIFT | VK_MENU | VK_CONTROL | VK_LWIN | VK_RWIN | VK_LSHIFT | VK_RSHIFT
            | VK_RCONTROL | VK_LCONTROL | VK_LMENU | VK_RMENU => None,
            _ => {
                if let Some(token) = common_named_key(vk) {
                    Some(token.to_owned())
                } else if is_printable_key(vk) {
                    const BUF_SIZE: usize = 8;
                    let mut key_buffer: Vec<uiohook_sys::platform::wchar_t> = vec![0; BUF_SIZE];
                    let str_count = unsafe {
                        uiohook_sys::platform::keycode_to_unicode(
                            keycode as u32,
                            key_buffer.as_mut_ptr(),
                            BUF_SIZE.try_into().unwrap(),
                        )
                    };
                    key_buffer.truncate(str_count.try_into().unwrap());
                    let key = OsString::from_wide(&key_buffer);
                    (!key.is_empty()).then(|| key.to_string_lossy().to_lowercase())
                } else {
                    None
                }
            }
        };

        let shift = event.mask & uiohook_sys::MASK_SHIFT as u16 != 0;
        let alt = event.mask & uiohook_sys::MASK_ALT as u16 != 0;
        let ctrl = event.mask & uiohook_sys::MASK_CTRL as u16 != 0;
        let meta = event.mask & uiohook_sys::MASK_META as u16 != 0;

        let mut curr_down = CURR_DOWN.lock().unwrap();
        curr_down.alt = alt;
        curr_down.shift = shift;
        curr_down.ctrl = ctrl;
        curr_down.meta = meta;
        if let Some(key) = key {
            if event.type_ == _event_type_EVENT_KEY_PRESSED {
                curr_down.keys.insert(key);
            } else {
                curr_down.keys.remove(&key);
            }
        }
        let keybinds = KEYBINDS.lock().unwrap();
        let active: HashSet<String> = keybinds
            .get_active_keybinds(&curr_down)
            .into_iter()
            .collect();
        let mut curr_active_keybinds = CURR_ACTIVE_KEYBINDS.lock().unwrap();
        let pressed_keybinds = active.difference(&curr_active_keybinds);
        let released_keybinds = curr_active_keybinds.difference(&active);
        for pressed in pressed_keybinds {
            TX.get()
                .unwrap()
                .send(KeybindTrigger::Pressed(pressed.clone()))
                .unwrap();
        }
        for released in released_keybinds {
            TX.get()
                .unwrap()
                .send(KeybindTrigger::Released(released.clone()))
                .unwrap();
        }
        curr_active_keybinds.clear();
        curr_active_keybinds.extend(active);
    }
}

pub(crate) fn set_keybinds_internal(keybinds: Vec<KeybindInfo>) -> Result<()> {
    let mut keybinds_mutex = KEYBINDS.lock().unwrap();
    keybinds_mutex.clear();
    keybinds.iter().for_each(|x| {
        if x.shortcut.is_some() {
            keybinds_mutex.register_keybind(
                Shortcut::from_string(x.shortcut.clone().unwrap()),
                x.id.clone(),
            )
        }
    });
    Ok(())
}

pub(crate) fn get_current_shortcut_internal() -> Result<String> {
    let down = CURR_DOWN.lock().unwrap();
    Ok(down.to_string())
}

fn common_named_key(vk: VIRTUAL_KEY) -> Option<&'static str> {
    Some(match vk {
        VK_PRIOR => "pageup",
        VK_NEXT => "pagedown",
        VK_HOME => "home",
        VK_END => "end",
        VK_INSERT => "insert",
        VK_DELETE => "delete",
        VK_ESCAPE => "escape",
        VK_RETURN => "enter",
        VK_BACK => "backspace",
        VK_TAB => "tab",
        VK_SPACE => "space",
        VK_UP => "up",
        VK_DOWN => "down",
        VK_LEFT => "left",
        VK_RIGHT => "right",
        VK_F1 => "f1",
        VK_F2 => "f2",
        VK_F3 => "f3",
        VK_F4 => "f4",
        VK_F5 => "f5",
        VK_F6 => "f6",
        VK_F7 => "f7",
        VK_F8 => "f8",
        VK_F9 => "f9",
        VK_F10 => "f10",
        VK_F11 => "f11",
        VK_F12 => "f12",
        _ => return None,
    })
}

fn is_printable_key(vk: VIRTUAL_KEY) -> bool {
    matches!(
        vk.0,
        0x30..=0x39 // 0-9
            | 0x41..=0x5a // A-Z
            | 0xba..=0xc0 // common OEM punctuation
            | 0xdb..=0xdf // common OEM punctuation
            | 0xe2 // VK_OEM_102
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_named_keys_use_stable_tokens() {
        let cases = [
            (VK_PRIOR, "pageup"),
            (VK_NEXT, "pagedown"),
            (VK_HOME, "home"),
            (VK_END, "end"),
            (VK_INSERT, "insert"),
            (VK_DELETE, "delete"),
            (VK_ESCAPE, "escape"),
            (VK_RETURN, "enter"),
            (VK_BACK, "backspace"),
            (VK_TAB, "tab"),
            (VK_SPACE, "space"),
            (VK_UP, "up"),
            (VK_DOWN, "down"),
            (VK_LEFT, "left"),
            (VK_RIGHT, "right"),
            (VK_F1, "f1"),
            (VK_F12, "f12"),
        ];

        for (vk, token) in cases {
            assert_eq!(common_named_key(vk), Some(token));
        }
    }

    #[test]
    fn printable_and_unsupported_keys_do_not_use_named_tokens() {
        for code in [0x30, 0x39, 0x41, 0x5a, 0xba, 0xc0, 0xdb, 0xdf, 0xe2] {
            let vk = VIRTUAL_KEY(code);
            assert!(is_printable_key(vk));
            assert_eq!(common_named_key(vk), None);
        }

        for code in [0x00, 0x01, 0xad, 0xae, 0xaf] {
            let vk = VIRTUAL_KEY(code);
            assert!(!is_printable_key(vk));
            assert_eq!(common_named_key(vk), None);
        }
    }
}
