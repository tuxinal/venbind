use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::{mpsc::Sender, Mutex};
use std::sync::{LazyLock, OnceLock};

use uiohook_sys::{
    _event_type_EVENT_KEY_PRESSED, _event_type_EVENT_KEY_RELEASED, _uiohook_event, hook_run,
    hook_set_dispatch_proc, UIOHOOK_SUCCESS,
};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_ADD, VK_APPS, VK_BACK, VK_BROWSER_BACK, VK_BROWSER_FAVORITES,
    VK_BROWSER_FORWARD, VK_BROWSER_HOME, VK_BROWSER_REFRESH, VK_BROWSER_SEARCH, VK_BROWSER_STOP,
    VK_CANCEL, VK_CAPITAL, VK_CLEAR, VK_CONTROL, VK_DECIMAL, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_EXECUTE, VK_F1, VK_F10, VK_F11, VK_F12, VK_F13, VK_F14, VK_F15, VK_F16, VK_F17,
    VK_F18, VK_F19, VK_F2, VK_F20, VK_F21, VK_F22, VK_F23, VK_F24, VK_F3, VK_F4, VK_F5, VK_F6,
    VK_F7, VK_F8, VK_F9, VK_HELP, VK_HOME, VK_INSERT, VK_LAUNCH_APP1, VK_LAUNCH_APP2,
    VK_LAUNCH_MAIL, VK_LAUNCH_MEDIA_SELECT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP, VK_MENU,
    VK_MULTIPLY, VK_NEXT, VK_NUMLOCK, VK_NUMPAD0, VK_NUMPAD1, VK_NUMPAD2, VK_NUMPAD3, VK_NUMPAD4,
    VK_NUMPAD5, VK_NUMPAD6, VK_NUMPAD7, VK_NUMPAD8, VK_NUMPAD9, VK_PAUSE, VK_PRINT, VK_PRIOR,
    VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SCROLL, VK_SELECT,
    VK_SEPARATOR, VK_SHIFT, VK_SLEEP, VK_SNAPSHOT, VK_SPACE, VK_SUBTRACT, VK_TAB, VK_UP,
    VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
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
static PRESSED_KEY_TOKENS: LazyLock<Mutex<HashMap<(u16, u16), String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
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
        let scancode = unsafe { event.data.keyboard.keycode };
        let vk = VIRTUAL_KEY(keycode);
        let physical_key = (keycode, scancode);
        let key: Option<String> = match vk {
            // Modifier keys are tracked via `event.mask`, never as a key token.
            VK_SHIFT | VK_MENU | VK_CONTROL | VK_LWIN | VK_RWIN | VK_LSHIFT | VK_RSHIFT
            | VK_RCONTROL | VK_LCONTROL | VK_LMENU | VK_RMENU => None,
            _ => {
                let mut pressed = PRESSED_KEY_TOKENS.lock().unwrap();
                cached_key_token(
                    &mut pressed,
                    physical_key,
                    event.type_ == _event_type_EVENT_KEY_PRESSED,
                    || resolve_pressed_key(vk, scancode, keycode),
                )
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
        // `dispatch_proc` is `extern "C"` and runs on libuiohook's hook thread, so
        // a panic here would unwind across the FFI boundary (undefined behaviour).
        // If the consumer has dropped the receiver the send simply fails -- ignore
        // it rather than `.unwrap()`-ing and aborting the host process.
        if let Some(tx) = TX.get() {
            for pressed in pressed_keybinds {
                let _ = tx.send(KeybindTrigger::Pressed(pressed.clone()));
            }
            for released in released_keybinds {
                let _ = tx.send(KeybindTrigger::Released(released.clone()));
            }
        }
        curr_active_keybinds.clear();
        curr_active_keybinds.extend(active);
    }
}

/// Returns one stable token for the lifetime of a physical key press.
///
/// The resolver is called only for the first press. Auto-repeat reuses the cached
/// value, and release removes and returns it without invoking the resolver. This
/// is important because libuiohook's unicode resolver mutates dead-key state.
fn cached_key_token<F>(
    pressed: &mut HashMap<(u16, u16), String>,
    physical_key: (u16, u16),
    is_pressed: bool,
    resolve: F,
) -> Option<String>
where
    F: FnOnce() -> Option<String>,
{
    if !is_pressed {
        return pressed.remove(&physical_key);
    }
    if let Some(token) = pressed.get(&physical_key) {
        return Some(token.clone());
    }
    let token = resolve();
    if let Some(token) = token.as_ref() {
        pressed.insert(physical_key, token.clone());
    }
    token
}

fn resolve_pressed_key(vk: VIRTUAL_KEY, scancode: u16, keycode: u16) -> Option<String> {
    // Named / non-printable keys -> a canonical, locale-independent token,
    // checked before the unicode path so ASCII control keys also remain stable.
    if let Some(token) = vk_to_token(vk, scancode) {
        return Some(token.to_owned());
    }

    // Printable keys -> their lowercased unicode character.
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

/// Maps a Windows virtual-key code for a named / non-printable key to venbind's
/// canonical, lowercase, locale-independent token (see `crate::structs::tokens`).
/// Returns `None` for printable keys (letters, digits, OEM punctuation), which
/// are matched by their unicode character instead. This is kept in lock-step with
/// `linux.rs::keysym_to_token`, so a consumer can register one string (e.g.
/// "ctrl+pageup", "f5") that matches identically on Windows and Linux/X11.
fn vk_to_token(vk: VIRTUAL_KEY, scancode: u16) -> Option<&'static str> {
    use crate::structs::tokens::*;

    // With NumLock off, Windows reports keypad digits as their navigation VKs.
    // libuiohook preserves whether the low-level event was extended in its
    // scancode. The non-extended form is the physical keypad key, so keep its
    // token stable across NumLock state.
    let numpad_navigation = match (vk, u32::from(scancode)) {
        (VK_INSERT, uiohook_sys::VC_INSERT) => Some(NUMPAD0),
        (VK_END, uiohook_sys::VC_END) => Some(NUMPAD1),
        (VK_DOWN, uiohook_sys::VC_DOWN) => Some(NUMPAD2),
        (VK_NEXT, uiohook_sys::VC_PAGE_DOWN) => Some(NUMPAD3),
        (VK_LEFT, uiohook_sys::VC_LEFT) => Some(NUMPAD4),
        (VK_CLEAR, uiohook_sys::VC_CLEAR) => Some(NUMPAD5),
        (VK_RIGHT, uiohook_sys::VC_RIGHT) => Some(NUMPAD6),
        (VK_HOME, uiohook_sys::VC_HOME) => Some(NUMPAD7),
        (VK_UP, uiohook_sys::VC_UP) => Some(NUMPAD8),
        (VK_PRIOR, uiohook_sys::VC_PAGE_UP) => Some(NUMPAD9),
        (VK_DELETE, uiohook_sys::VC_DELETE) => Some(NUMPAD_DECIMAL),
        _ => None,
    };
    if numpad_navigation.is_some() {
        return numpad_navigation;
    }

    Some(match vk {
        VK_PRIOR => PAGE_UP,
        VK_NEXT => PAGE_DOWN,
        VK_HOME => HOME,
        VK_END => END,
        VK_INSERT => INSERT,
        VK_DELETE => DELETE,
        VK_ESCAPE => ESCAPE,
        VK_RETURN if u32::from(scancode) == uiohook_sys::VC_KP_ENTER => NUMPAD_ENTER,
        VK_RETURN => ENTER,
        VK_BACK => BACKSPACE,
        VK_TAB => TAB,
        VK_SPACE => SPACE,
        VK_UP => UP,
        VK_DOWN => DOWN,
        VK_LEFT => LEFT,
        VK_RIGHT => RIGHT,
        VK_CAPITAL => CAPS_LOCK,
        VK_NUMLOCK => NUM_LOCK,
        VK_SCROLL => SCROLL_LOCK,
        VK_SNAPSHOT => PRINT_SCREEN,
        VK_PAUSE => PAUSE,
        VK_APPS => MENU,
        VK_CANCEL => CANCEL,
        VK_CLEAR => CLEAR,
        VK_SELECT => SELECT,
        VK_PRINT => PRINT,
        VK_EXECUTE => EXECUTE,
        VK_HELP => HELP,
        VK_SLEEP => SLEEP,
        VK_F1 => F1,
        VK_F2 => F2,
        VK_F3 => F3,
        VK_F4 => F4,
        VK_F5 => F5,
        VK_F6 => F6,
        VK_F7 => F7,
        VK_F8 => F8,
        VK_F9 => F9,
        VK_F10 => F10,
        VK_F11 => F11,
        VK_F12 => F12,
        VK_F13 => F13,
        VK_F14 => F14,
        VK_F15 => F15,
        VK_F16 => F16,
        VK_F17 => F17,
        VK_F18 => F18,
        VK_F19 => F19,
        VK_F20 => F20,
        VK_F21 => F21,
        VK_F22 => F22,
        VK_F23 => F23,
        VK_F24 => F24,
        VK_NUMPAD0 => NUMPAD0,
        VK_NUMPAD1 => NUMPAD1,
        VK_NUMPAD2 => NUMPAD2,
        VK_NUMPAD3 => NUMPAD3,
        VK_NUMPAD4 => NUMPAD4,
        VK_NUMPAD5 => NUMPAD5,
        VK_NUMPAD6 => NUMPAD6,
        VK_NUMPAD7 => NUMPAD7,
        VK_NUMPAD8 => NUMPAD8,
        VK_NUMPAD9 => NUMPAD9,
        VK_ADD => NUMPAD_ADD,
        VK_SUBTRACT => NUMPAD_SUBTRACT,
        VK_MULTIPLY => NUMPAD_MULTIPLY,
        VK_DIVIDE => NUMPAD_DIVIDE,
        VK_DECIMAL => NUMPAD_DECIMAL,
        VK_SEPARATOR => NUMPAD_SEPARATOR,
        VK_VOLUME_MUTE => VOLUME_MUTE,
        VK_VOLUME_DOWN => VOLUME_DOWN,
        VK_VOLUME_UP => VOLUME_UP,
        VK_MEDIA_NEXT_TRACK => MEDIA_NEXT_TRACK,
        VK_MEDIA_PREV_TRACK => MEDIA_PREV_TRACK,
        VK_MEDIA_STOP => MEDIA_STOP,
        VK_MEDIA_PLAY_PAUSE => MEDIA_PLAY_PAUSE,
        VK_BROWSER_BACK => BROWSER_BACK,
        VK_BROWSER_FORWARD => BROWSER_FORWARD,
        VK_BROWSER_REFRESH => BROWSER_REFRESH,
        VK_BROWSER_STOP => BROWSER_STOP,
        VK_BROWSER_SEARCH => BROWSER_SEARCH,
        VK_BROWSER_FAVORITES => BROWSER_FAVORITES,
        VK_BROWSER_HOME => BROWSER_HOME,
        VK_LAUNCH_MAIL => LAUNCH_MAIL,
        VK_LAUNCH_MEDIA_SELECT => LAUNCH_MEDIA,
        VK_LAUNCH_APP1 => LAUNCH_APP1,
        VK_LAUNCH_APP2 => LAUNCH_APP2,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structs::tokens;
    use std::cell::Cell;

    #[test]
    fn named_keys_map_to_canonical_tokens() {
        assert_eq!(vk_to_token(VK_PRIOR, 0), Some(tokens::PAGE_UP));
        assert_eq!(vk_to_token(VK_NEXT, 0), Some(tokens::PAGE_DOWN));
        assert_eq!(vk_to_token(VK_SPACE, 0), Some(tokens::SPACE));
        assert_eq!(
            vk_to_token(VK_RETURN, uiohook_sys::VC_ENTER as u16),
            Some(tokens::ENTER)
        );
        assert_eq!(vk_to_token(VK_F5, 0), Some(tokens::F5));
        assert_eq!(vk_to_token(VK_NUMPAD7, 0), Some(tokens::NUMPAD7));
    }

    #[test]
    fn extended_named_keys_map_to_canonical_tokens() {
        let cases = [
            (VK_PRIOR, tokens::PAGE_UP),
            (VK_NEXT, tokens::PAGE_DOWN),
            (VK_HOME, tokens::HOME),
            (VK_END, tokens::END),
            (VK_INSERT, tokens::INSERT),
            (VK_DELETE, tokens::DELETE),
            (VK_ESCAPE, tokens::ESCAPE),
            (VK_BACK, tokens::BACKSPACE),
            (VK_TAB, tokens::TAB),
            (VK_SPACE, tokens::SPACE),
            (VK_UP, tokens::UP),
            (VK_DOWN, tokens::DOWN),
            (VK_LEFT, tokens::LEFT),
            (VK_RIGHT, tokens::RIGHT),
            (VK_CAPITAL, tokens::CAPS_LOCK),
            (VK_NUMLOCK, tokens::NUM_LOCK),
            (VK_SCROLL, tokens::SCROLL_LOCK),
            (VK_SNAPSHOT, tokens::PRINT_SCREEN),
            (VK_PAUSE, tokens::PAUSE),
            (VK_APPS, tokens::MENU),
            (VK_CANCEL, tokens::CANCEL),
            (VK_CLEAR, tokens::CLEAR),
            (VK_SELECT, tokens::SELECT),
            (VK_PRINT, tokens::PRINT),
            (VK_EXECUTE, tokens::EXECUTE),
            (VK_HELP, tokens::HELP),
            (VK_SLEEP, tokens::SLEEP),
            (VK_F1, tokens::F1),
            (VK_F2, tokens::F2),
            (VK_F3, tokens::F3),
            (VK_F4, tokens::F4),
            (VK_F5, tokens::F5),
            (VK_F6, tokens::F6),
            (VK_F7, tokens::F7),
            (VK_F8, tokens::F8),
            (VK_F9, tokens::F9),
            (VK_F10, tokens::F10),
            (VK_F11, tokens::F11),
            (VK_F12, tokens::F12),
            (VK_F13, tokens::F13),
            (VK_F14, tokens::F14),
            (VK_F15, tokens::F15),
            (VK_F16, tokens::F16),
            (VK_F17, tokens::F17),
            (VK_F18, tokens::F18),
            (VK_F19, tokens::F19),
            (VK_F20, tokens::F20),
            (VK_F21, tokens::F21),
            (VK_F22, tokens::F22),
            (VK_F23, tokens::F23),
            (VK_F24, tokens::F24),
            (VK_NUMPAD0, tokens::NUMPAD0),
            (VK_NUMPAD1, tokens::NUMPAD1),
            (VK_NUMPAD2, tokens::NUMPAD2),
            (VK_NUMPAD3, tokens::NUMPAD3),
            (VK_NUMPAD4, tokens::NUMPAD4),
            (VK_NUMPAD5, tokens::NUMPAD5),
            (VK_NUMPAD6, tokens::NUMPAD6),
            (VK_NUMPAD7, tokens::NUMPAD7),
            (VK_NUMPAD8, tokens::NUMPAD8),
            (VK_NUMPAD9, tokens::NUMPAD9),
            (VK_ADD, tokens::NUMPAD_ADD),
            (VK_SUBTRACT, tokens::NUMPAD_SUBTRACT),
            (VK_MULTIPLY, tokens::NUMPAD_MULTIPLY),
            (VK_DIVIDE, tokens::NUMPAD_DIVIDE),
            (VK_DECIMAL, tokens::NUMPAD_DECIMAL),
            (VK_SEPARATOR, tokens::NUMPAD_SEPARATOR),
            (VK_VOLUME_MUTE, tokens::VOLUME_MUTE),
            (VK_VOLUME_DOWN, tokens::VOLUME_DOWN),
            (VK_VOLUME_UP, tokens::VOLUME_UP),
            (VK_MEDIA_NEXT_TRACK, tokens::MEDIA_NEXT_TRACK),
            (VK_MEDIA_PREV_TRACK, tokens::MEDIA_PREV_TRACK),
            (VK_MEDIA_STOP, tokens::MEDIA_STOP),
            (VK_MEDIA_PLAY_PAUSE, tokens::MEDIA_PLAY_PAUSE),
            (VK_BROWSER_BACK, tokens::BROWSER_BACK),
            (VK_BROWSER_FORWARD, tokens::BROWSER_FORWARD),
            (VK_BROWSER_REFRESH, tokens::BROWSER_REFRESH),
            (VK_BROWSER_STOP, tokens::BROWSER_STOP),
            (VK_BROWSER_SEARCH, tokens::BROWSER_SEARCH),
            (VK_BROWSER_FAVORITES, tokens::BROWSER_FAVORITES),
            (VK_BROWSER_HOME, tokens::BROWSER_HOME),
            (VK_LAUNCH_MAIL, tokens::LAUNCH_MAIL),
            (VK_LAUNCH_MEDIA_SELECT, tokens::LAUNCH_MEDIA),
            (VK_LAUNCH_APP1, tokens::LAUNCH_APP1),
            (VK_LAUNCH_APP2, tokens::LAUNCH_APP2),
        ];
        for (vk, token) in cases {
            assert_eq!(vk_to_token(vk, 0), Some(token), "VK {:#x}", vk.0);
        }
        assert_eq!(
            vk_to_token(VK_RETURN, uiohook_sys::VC_ENTER as u16),
            Some(tokens::ENTER)
        );
        assert_eq!(
            vk_to_token(VK_RETURN, uiohook_sys::VC_KP_ENTER as u16),
            Some(tokens::NUMPAD_ENTER)
        );
    }

    #[test]
    fn modifier_and_unknown_virtual_keys_are_not_named_tokens() {
        for vk in [
            VK_SHIFT,
            VK_LSHIFT,
            VK_RSHIFT,
            VK_CONTROL,
            VK_LCONTROL,
            VK_RCONTROL,
            VK_MENU,
            VK_LMENU,
            VK_RMENU,
            VK_LWIN,
            VK_RWIN,
            VIRTUAL_KEY(0),
        ] {
            assert_eq!(vk_to_token(vk, 0), None, "VK {:#x}", vk.0);
        }
    }

    #[test]
    fn numpad_tokens_are_stable_across_numlock_state() {
        let cases = [
            (VK_INSERT, uiohook_sys::VC_INSERT, tokens::NUMPAD0),
            (VK_END, uiohook_sys::VC_END, tokens::NUMPAD1),
            (VK_DOWN, uiohook_sys::VC_DOWN, tokens::NUMPAD2),
            (VK_NEXT, uiohook_sys::VC_PAGE_DOWN, tokens::NUMPAD3),
            (VK_LEFT, uiohook_sys::VC_LEFT, tokens::NUMPAD4),
            (VK_CLEAR, uiohook_sys::VC_CLEAR, tokens::NUMPAD5),
            (VK_RIGHT, uiohook_sys::VC_RIGHT, tokens::NUMPAD6),
            (VK_HOME, uiohook_sys::VC_HOME, tokens::NUMPAD7),
            (VK_UP, uiohook_sys::VC_UP, tokens::NUMPAD8),
            (VK_PRIOR, uiohook_sys::VC_PAGE_UP, tokens::NUMPAD9),
            (VK_DELETE, uiohook_sys::VC_DELETE, tokens::NUMPAD_DECIMAL),
        ];
        for (vk, scancode, token) in cases {
            assert_eq!(vk_to_token(vk, scancode as u16), Some(token));
        }
        assert_eq!(
            vk_to_token(VK_RETURN, uiohook_sys::VC_KP_ENTER as u16),
            Some(tokens::NUMPAD_ENTER)
        );
    }

    #[test]
    fn press_repeat_and_release_share_one_resolved_token() {
        let mut pressed = HashMap::new();
        let resolver_calls = Cell::new(0);
        let physical_key = (VK_F5.0, uiohook_sys::VC_F5 as u16);

        let first = cached_key_token(&mut pressed, physical_key, true, || {
            resolver_calls.set(resolver_calls.get() + 1);
            Some(tokens::F5.to_owned())
        });
        let repeat = cached_key_token(&mut pressed, physical_key, true, || {
            resolver_calls.set(resolver_calls.get() + 1);
            Some("wrong-repeat-token".to_owned())
        });
        let released = cached_key_token(&mut pressed, physical_key, false, || {
            resolver_calls.set(resolver_calls.get() + 1);
            Some("wrong-release-token".to_owned())
        });

        assert_eq!(first.as_deref(), Some(tokens::F5));
        assert_eq!(repeat.as_deref(), Some(tokens::F5));
        assert_eq!(released.as_deref(), Some(tokens::F5));
        assert_eq!(resolver_calls.get(), 1);
        assert!(pressed.is_empty());
    }

    #[test]
    fn release_without_a_press_does_not_resolve_or_stick() {
        let mut pressed = HashMap::new();
        let resolver_called = Cell::new(false);
        let released = cached_key_token(&mut pressed, (0x41, 0x1e), false, || {
            resolver_called.set(true);
            Some("a".to_owned())
        });
        assert_eq!(released, None);
        assert!(!resolver_called.get());
        assert!(pressed.is_empty());
    }
}
