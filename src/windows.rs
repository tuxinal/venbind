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
static EVENT_STATE: LazyLock<Mutex<WindowsEventState>> =
    LazyLock::new(|| Mutex::new(WindowsEventState::default()));
static TX: OnceLock<Sender<KeybindTrigger>> = OnceLock::new();

/// All deterministic state owned by the Windows keyboard event path.
///
/// Keeping this state together lets the real libuiohook event processor be
/// exercised without starting a global hook or requiring an interactive desktop.
struct WindowsEventState {
    curr_down: Shortcut,
    curr_active_keybinds: HashSet<KeybindId>,
    pressed_key_tokens: HashMap<(u16, u16), String>,
}

impl Default for WindowsEventState {
    fn default() -> Self {
        Self {
            curr_down: Shortcut {
                shift: false,
                alt: false,
                ctrl: false,
                meta: false,
                keys: HashSet::new(),
            },
            curr_active_keybinds: HashSet::new(),
            pressed_key_tokens: HashMap::new(),
        }
    }
}

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
    if event_ref.is_null() {
        return;
    }

    // This callback runs on libuiohook's C hook thread. Catch any Rust panic so
    // it can never unwind across the FFI boundary. Poisoned locks are handled as
    // a dropped event for the same reason.
    let _ = std::panic::catch_unwind(|| {
        let event = unsafe { &*event_ref };
        let triggers = {
            let Ok(keybinds) = KEYBINDS.lock() else {
                return;
            };
            let Ok(mut state) = EVENT_STATE.lock() else {
                return;
            };
            process_keyboard_event(&mut state, &keybinds, event, resolve_pressed_key)
        };

        if let Some(tx) = TX.get() {
            for trigger in triggers {
                // The receiver belongs to the embedding application and may have
                // been dropped; that must not abort the host process.
                let _ = tx.send(trigger);
            }
        }
    });
}

/// Process one production-format libuiohook event without starting a hook.
///
/// The resolver is injectable only so printable-key cache behavior can be tested
/// independently of the runner's active keyboard layout. Production passes
/// [`resolve_pressed_key`] directly, and named-key tests do the same.
fn process_keyboard_event<F>(
    state: &mut WindowsEventState,
    keybinds: &Keybinds,
    event: &_uiohook_event,
    resolve: F,
) -> Vec<KeybindTrigger>
where
    F: FnOnce(VIRTUAL_KEY, u16, u16) -> Option<String>,
{
    let is_pressed = match event.type_ {
        _event_type_EVENT_KEY_PRESSED => true,
        _event_type_EVENT_KEY_RELEASED => false,
        _ => return Vec::new(),
    };

    let (keycode, scancode) = unsafe { (event.data.keyboard.rawcode, event.data.keyboard.keycode) };
    let vk = VIRTUAL_KEY(keycode);
    let physical_key = (keycode, scancode);
    let key = match vk {
        // Modifier keys are tracked via `event.mask`, never as a key token.
        VK_SHIFT | VK_MENU | VK_CONTROL | VK_LWIN | VK_RWIN | VK_LSHIFT | VK_RSHIFT
        | VK_RCONTROL | VK_LCONTROL | VK_LMENU | VK_RMENU => None,
        _ => cached_key_token(
            &mut state.pressed_key_tokens,
            physical_key,
            is_pressed,
            || resolve(vk, scancode, keycode),
        ),
    };

    state.curr_down.shift = event.mask & uiohook_sys::MASK_SHIFT as u16 != 0;
    state.curr_down.alt = event.mask & uiohook_sys::MASK_ALT as u16 != 0;
    state.curr_down.ctrl = event.mask & uiohook_sys::MASK_CTRL as u16 != 0;
    state.curr_down.meta = event.mask & uiohook_sys::MASK_META as u16 != 0;
    if let Some(key) = key {
        if is_pressed {
            state.curr_down.keys.insert(key);
        } else {
            state.curr_down.keys.remove(&key);
        }
    }

    let active: HashSet<KeybindId> = keybinds
        .get_active_keybinds(&state.curr_down)
        .into_iter()
        .collect();
    let mut pressed: Vec<KeybindId> = active
        .difference(&state.curr_active_keybinds)
        .cloned()
        .collect();
    let mut released: Vec<KeybindId> = state
        .curr_active_keybinds
        .difference(&active)
        .cloned()
        .collect();
    pressed.sort_unstable();
    released.sort_unstable();

    state.curr_active_keybinds = active;
    pressed
        .into_iter()
        .map(KeybindTrigger::Pressed)
        .chain(released.into_iter().map(KeybindTrigger::Released))
        .collect()
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

    // Keep unsupported non-character VK groups bounded. Common printable keys
    // are alphanumerics plus the layout-dependent OEM punctuation keys; IME,
    // packet, gamepad, reserved, and legacy terminal ranges are not sent through
    // libuiohook's layout resolver.
    if !is_printable_vk(vk) {
        return None;
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
    let str_count = usize::try_from(str_count).unwrap_or(BUF_SIZE).min(BUF_SIZE);
    key_buffer.truncate(str_count);
    let key = OsString::from_wide(&key_buffer);
    (!key.is_empty()).then(|| key.to_string_lossy().to_lowercase())
}

fn is_printable_vk(vk: VIRTUAL_KEY) -> bool {
    matches!(
        vk.0,
        0x30..=0x39 // 0-9
            | 0x41..=0x5a // A-Z
            | 0xba..=0xc0 // common OEM punctuation
            | 0xdb..=0xdf // common OEM punctuation
            | 0xe2 // VK_OEM_102
    )
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
    let state = EVENT_STATE.lock().unwrap();
    Ok(state.curr_down.to_string())
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

    const LLKHF_EXTENDED: u32 = 0x01;

    fn libuiohook_scancode(vk: VIRTUAL_KEY, extended: bool) -> u16 {
        unsafe {
            uiohook_sys::platform::keycode_to_scancode(
                u32::from(vk.0),
                if extended { LLKHF_EXTENDED } else { 0 },
            )
        }
    }

    fn keyboard_event(type_: i32, vk: VIRTUAL_KEY, scancode: u16, mask: u16) -> _uiohook_event {
        let mut event: _uiohook_event = unsafe { std::mem::zeroed() };
        event.type_ = type_;
        event.mask = mask;
        unsafe {
            event.data.keyboard.keycode = scancode;
            event.data.keyboard.rawcode = vk.0;
            event.data.keyboard.keychar = uiohook_sys::CHAR_UNDEFINED as u16;
        }
        event
    }

    fn keybind(shortcut: &str) -> Keybinds {
        let mut keybinds = Keybinds::default();
        keybinds.register_keybind(
            Shortcut::from_string(shortcut.to_owned()),
            "binding".to_owned(),
        );
        keybinds
    }

    fn assert_named_press_and_release(vk: VIRTUAL_KEY, extended: bool, token: &str) {
        let keybinds = keybind(token);
        let mut state = WindowsEventState::default();
        let scancode = libuiohook_scancode(vk, extended);
        let press = keyboard_event(_event_type_EVENT_KEY_PRESSED, vk, scancode, 0);
        let release = keyboard_event(_event_type_EVENT_KEY_RELEASED, vk, scancode, 0);

        assert_eq!(
            process_keyboard_event(&mut state, &keybinds, &press, resolve_pressed_key),
            vec![KeybindTrigger::Pressed("binding".to_owned())],
            "press VK {:#x}, scancode {scancode:#x}",
            vk.0
        );
        assert!(state.curr_down.keys.contains(token));
        assert_eq!(
            process_keyboard_event(&mut state, &keybinds, &release, resolve_pressed_key),
            vec![KeybindTrigger::Released("binding".to_owned())],
            "release VK {:#x}, scancode {scancode:#x}",
            vk.0
        );
        assert!(state.curr_down.keys.is_empty());
        assert!(state.pressed_key_tokens.is_empty());
    }

    #[test]
    fn production_events_cover_navigation_editing_whitespace_arrows_and_f1_to_f24() {
        let cases = [
            (VK_PRIOR, true, tokens::PAGE_UP),
            (VK_NEXT, true, tokens::PAGE_DOWN),
            (VK_HOME, true, tokens::HOME),
            (VK_END, true, tokens::END),
            (VK_INSERT, true, tokens::INSERT),
            (VK_DELETE, true, tokens::DELETE),
            (VK_ESCAPE, false, tokens::ESCAPE),
            (VK_RETURN, false, tokens::ENTER),
            (VK_BACK, false, tokens::BACKSPACE),
            (VK_TAB, false, tokens::TAB),
            (VK_SPACE, false, tokens::SPACE),
            (VK_UP, true, tokens::UP),
            (VK_DOWN, true, tokens::DOWN),
            (VK_LEFT, true, tokens::LEFT),
            (VK_RIGHT, true, tokens::RIGHT),
            (VK_F1, false, tokens::F1),
            (VK_F2, false, tokens::F2),
            (VK_F3, false, tokens::F3),
            (VK_F4, false, tokens::F4),
            (VK_F5, false, tokens::F5),
            (VK_F6, false, tokens::F6),
            (VK_F7, false, tokens::F7),
            (VK_F8, false, tokens::F8),
            (VK_F9, false, tokens::F9),
            (VK_F10, false, tokens::F10),
            (VK_F11, false, tokens::F11),
            (VK_F12, false, tokens::F12),
            (VK_F13, false, tokens::F13),
            (VK_F14, false, tokens::F14),
            (VK_F15, false, tokens::F15),
            (VK_F16, false, tokens::F16),
            (VK_F17, false, tokens::F17),
            (VK_F18, false, tokens::F18),
            (VK_F19, false, tokens::F19),
            (VK_F20, false, tokens::F20),
            (VK_F21, false, tokens::F21),
            (VK_F22, false, tokens::F22),
            (VK_F23, false, tokens::F23),
            (VK_F24, false, tokens::F24),
        ];
        for (vk, extended, token) in cases {
            assert_named_press_and_release(vk, extended, token);
        }
    }

    #[test]
    fn production_events_cover_locks_keypad_and_common_system_keys() {
        let cases = [
            (VK_CAPITAL, false, tokens::CAPS_LOCK),
            (VK_NUMLOCK, true, tokens::NUM_LOCK),
            (VK_SCROLL, false, tokens::SCROLL_LOCK),
            (VK_SNAPSHOT, true, tokens::PRINT_SCREEN),
            (VK_PAUSE, false, tokens::PAUSE),
            (VK_APPS, true, tokens::MENU),
            (VK_CANCEL, false, tokens::CANCEL),
            (VK_SELECT, false, tokens::SELECT),
            (VK_PRINT, false, tokens::PRINT),
            (VK_EXECUTE, false, tokens::EXECUTE),
            (VK_HELP, false, tokens::HELP),
            (VK_SLEEP, false, tokens::SLEEP),
            (VK_NUMPAD0, false, tokens::NUMPAD0),
            (VK_NUMPAD1, false, tokens::NUMPAD1),
            (VK_NUMPAD2, false, tokens::NUMPAD2),
            (VK_NUMPAD3, false, tokens::NUMPAD3),
            (VK_NUMPAD4, false, tokens::NUMPAD4),
            (VK_NUMPAD5, false, tokens::NUMPAD5),
            (VK_NUMPAD6, false, tokens::NUMPAD6),
            (VK_NUMPAD7, false, tokens::NUMPAD7),
            (VK_NUMPAD8, false, tokens::NUMPAD8),
            (VK_NUMPAD9, false, tokens::NUMPAD9),
            (VK_ADD, false, tokens::NUMPAD_ADD),
            (VK_SUBTRACT, false, tokens::NUMPAD_SUBTRACT),
            (VK_MULTIPLY, false, tokens::NUMPAD_MULTIPLY),
            (VK_DIVIDE, true, tokens::NUMPAD_DIVIDE),
            (VK_DECIMAL, false, tokens::NUMPAD_DECIMAL),
            (VK_SEPARATOR, false, tokens::NUMPAD_SEPARATOR),
        ];
        for (vk, extended, token) in cases {
            assert_named_press_and_release(vk, extended, token);
        }
    }

    #[test]
    fn production_events_cover_volume_media_browser_and_launch_keys() {
        let cases = [
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
            assert_named_press_and_release(vk, false, token);
        }
    }

    #[test]
    fn vendored_scancode_translation_distinguishes_enter_and_navigation_sources() {
        assert_eq!(
            libuiohook_scancode(VK_RETURN, false),
            uiohook_sys::VC_ENTER as u16
        );
        assert_eq!(
            libuiohook_scancode(VK_RETURN, true),
            uiohook_sys::VC_KP_ENTER as u16
        );

        let navigation = [
            (VK_INSERT, uiohook_sys::VC_INSERT, uiohook_sys::VC_KP_INSERT),
            (VK_END, uiohook_sys::VC_END, uiohook_sys::VC_KP_END),
            (VK_DOWN, uiohook_sys::VC_DOWN, uiohook_sys::VC_KP_DOWN),
            (
                VK_NEXT,
                uiohook_sys::VC_PAGE_DOWN,
                uiohook_sys::VC_KP_PAGE_DOWN,
            ),
            (VK_LEFT, uiohook_sys::VC_LEFT, uiohook_sys::VC_KP_LEFT),
            (VK_RIGHT, uiohook_sys::VC_RIGHT, uiohook_sys::VC_KP_RIGHT),
            (VK_HOME, uiohook_sys::VC_HOME, uiohook_sys::VC_KP_HOME),
            (VK_UP, uiohook_sys::VC_UP, uiohook_sys::VC_KP_UP),
            (
                VK_PRIOR,
                uiohook_sys::VC_PAGE_UP,
                uiohook_sys::VC_KP_PAGE_UP,
            ),
            (VK_DELETE, uiohook_sys::VC_DELETE, uiohook_sys::VC_KP_DELETE),
        ];
        for (vk, non_extended, extended) in navigation {
            assert_eq!(libuiohook_scancode(vk, false), non_extended as u16);
            assert_eq!(libuiohook_scancode(vk, true), extended as u16);
        }

        // The vendored helper does not branch on LLKHF_EXTENDED for VK_CLEAR.
        assert_eq!(
            libuiohook_scancode(VK_CLEAR, false),
            uiohook_sys::VC_CLEAR as u16
        );
        assert_eq!(
            libuiohook_scancode(VK_CLEAR, true),
            uiohook_sys::VC_CLEAR as u16
        );
    }

    #[test]
    fn production_events_distinguish_main_enter_numpad_enter_and_numlock_off_keypad() {
        assert_named_press_and_release(VK_RETURN, false, tokens::ENTER);
        assert_named_press_and_release(VK_RETURN, true, tokens::NUMPAD_ENTER);

        let cases = [
            (VK_INSERT, tokens::NUMPAD0, tokens::INSERT),
            (VK_END, tokens::NUMPAD1, tokens::END),
            (VK_DOWN, tokens::NUMPAD2, tokens::DOWN),
            (VK_NEXT, tokens::NUMPAD3, tokens::PAGE_DOWN),
            (VK_LEFT, tokens::NUMPAD4, tokens::LEFT),
            (VK_RIGHT, tokens::NUMPAD6, tokens::RIGHT),
            (VK_HOME, tokens::NUMPAD7, tokens::HOME),
            (VK_UP, tokens::NUMPAD8, tokens::UP),
            (VK_PRIOR, tokens::NUMPAD9, tokens::PAGE_UP),
            (VK_DELETE, tokens::NUMPAD_DECIMAL, tokens::DELETE),
        ];
        for (vk, keypad_token, dedicated_token) in cases {
            assert_named_press_and_release(vk, false, keypad_token);
            assert_named_press_and_release(vk, true, dedicated_token);
        }

        // VK_CLEAR always has VC_CLEAR in this vendored helper, so the only
        // reliable physical interpretation is Numpad 5 with NumLock off.
        assert_named_press_and_release(VK_CLEAR, false, tokens::NUMPAD5);
    }

    #[test]
    fn production_events_track_all_modifier_masks_without_sided_tokens() {
        let cases = [
            (VK_LSHIFT, false, uiohook_sys::MASK_SHIFT_L as u16, "shift"),
            (VK_RSHIFT, false, uiohook_sys::MASK_SHIFT_R as u16, "shift"),
            (VK_LMENU, false, uiohook_sys::MASK_ALT_L as u16, "alt"),
            (VK_RMENU, true, uiohook_sys::MASK_ALT_R as u16, "alt"),
            (VK_LCONTROL, false, uiohook_sys::MASK_CTRL_L as u16, "ctrl"),
            (VK_RCONTROL, true, uiohook_sys::MASK_CTRL_R as u16, "ctrl"),
            (VK_LWIN, true, uiohook_sys::MASK_META_L as u16, "meta"),
            (VK_RWIN, true, uiohook_sys::MASK_META_R as u16, "meta"),
        ];
        for (vk, extended, mask, modifier) in cases {
            let keybinds = keybind(&format!("{modifier}+f5"));
            let mut state = WindowsEventState::default();
            let modifier_scancode = libuiohook_scancode(vk, extended);
            let modifier_press =
                keyboard_event(_event_type_EVENT_KEY_PRESSED, vk, modifier_scancode, mask);
            assert!(process_keyboard_event(
                &mut state,
                &keybinds,
                &modifier_press,
                resolve_pressed_key
            )
            .is_empty());
            assert!(state.curr_down.keys.is_empty());
            assert!(state.pressed_key_tokens.is_empty());

            let f5_scancode = libuiohook_scancode(VK_F5, false);
            let f5_press = keyboard_event(_event_type_EVENT_KEY_PRESSED, VK_F5, f5_scancode, mask);
            assert_eq!(
                process_keyboard_event(&mut state, &keybinds, &f5_press, resolve_pressed_key),
                vec![KeybindTrigger::Pressed("binding".to_owned())]
            );
            let f5_release =
                keyboard_event(_event_type_EVENT_KEY_RELEASED, VK_F5, f5_scancode, mask);
            assert_eq!(
                process_keyboard_event(&mut state, &keybinds, &f5_release, resolve_pressed_key),
                vec![KeybindTrigger::Released("binding".to_owned())]
            );
            let modifier_release =
                keyboard_event(_event_type_EVENT_KEY_RELEASED, vk, modifier_scancode, 0);
            assert!(process_keyboard_event(
                &mut state,
                &keybinds,
                &modifier_release,
                resolve_pressed_key
            )
            .is_empty());
        }
    }

    #[test]
    fn production_events_cache_press_token_through_repeat_and_release() {
        let keybinds = keybind("press-time-token");
        let mut state = WindowsEventState::default();
        let resolver_calls = Cell::new(0);
        let scancode = libuiohook_scancode(VIRTUAL_KEY(0x41), false);
        let press = keyboard_event(
            _event_type_EVENT_KEY_PRESSED,
            VIRTUAL_KEY(0x41),
            scancode,
            0,
        );
        let release = keyboard_event(
            _event_type_EVENT_KEY_RELEASED,
            VIRTUAL_KEY(0x41),
            scancode,
            0,
        );

        let first = process_keyboard_event(&mut state, &keybinds, &press, |_, _, _| {
            resolver_calls.set(resolver_calls.get() + 1);
            Some("press-time-token".to_owned())
        });
        let repeat = process_keyboard_event(&mut state, &keybinds, &press, |_, _, _| {
            resolver_calls.set(resolver_calls.get() + 1);
            Some("wrong-repeat-token".to_owned())
        });
        let released = process_keyboard_event(&mut state, &keybinds, &release, |_, _, _| {
            resolver_calls.set(resolver_calls.get() + 1);
            Some("wrong-release-token".to_owned())
        });

        assert_eq!(first, vec![KeybindTrigger::Pressed("binding".to_owned())]);
        assert!(
            repeat.is_empty(),
            "auto-repeat must not retrigger a keybind"
        );
        assert_eq!(
            released,
            vec![KeybindTrigger::Released("binding".to_owned())]
        );
        assert_eq!(resolver_calls.get(), 1);
        assert!(state.curr_down.keys.is_empty());
        assert!(state.pressed_key_tokens.is_empty());
    }

    #[test]
    fn production_events_ignore_unknown_and_unsupported_virtual_keys() {
        let keybinds = keybind("unexpected");
        for vk in [
            VIRTUAL_KEY(0x00), // undefined
            VIRTUAL_KEY(0xe5), // VK_PROCESSKEY
            VIRTUAL_KEY(0xe7), // VK_PACKET
            VIRTUAL_KEY(0xc3), // VK_GAMEPAD_A
            VIRTUAL_KEY(0xf6), // VK_ATTN
        ] {
            let mut state = WindowsEventState::default();
            let scancode = libuiohook_scancode(vk, false);
            let press = keyboard_event(_event_type_EVENT_KEY_PRESSED, vk, scancode, 0);
            let release = keyboard_event(_event_type_EVENT_KEY_RELEASED, vk, scancode, 0);
            assert!(
                process_keyboard_event(&mut state, &keybinds, &press, resolve_pressed_key)
                    .is_empty()
            );
            assert!(
                process_keyboard_event(&mut state, &keybinds, &release, resolve_pressed_key)
                    .is_empty()
            );
            assert!(state.curr_down.keys.is_empty());
            assert!(state.pressed_key_tokens.is_empty());
        }
    }
}
