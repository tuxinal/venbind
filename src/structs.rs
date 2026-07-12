use std::collections::HashSet;

pub type KeybindId = String;

#[cfg(feature = "node")]
use napi_derive::napi;

#[derive(Default)]
pub struct Keybinds {
    keybinds: Vec<(Shortcut, KeybindId)>,
}

#[cfg_attr(feature = "node", napi(object))]
pub struct KeybindInfo {
    pub id: KeybindId,
    pub name: Option<String>,
    pub shortcut: Option<String>,
}

pub enum KeybindTrigger {
    Pressed(KeybindId),
    Released(KeybindId),
}

/// Canonical key tokens shared by every platform backend.
///
/// The press side (`windows.rs::vk_to_token` / `linux.rs::keysym_to_token`) maps
/// each named / non-printable key to one of these strings, and
/// [`Shortcut::from_string`] lowercases the registered shortcut. Because the
/// tokens are lowercase and identical across platforms, a consumer can register a
/// single string (e.g. `"ctrl+pageup"`, `"f5"`) that matches on Windows and
/// Linux/X11 alike. Printable keys (letters, digits, punctuation) are matched by
/// their lowercased unicode character and are deliberately NOT listed here.
pub(crate) mod tokens {
    pub const PAGE_UP: &str = "pageup";
    pub const PAGE_DOWN: &str = "pagedown";
    pub const HOME: &str = "home";
    pub const END: &str = "end";
    pub const INSERT: &str = "insert";
    pub const DELETE: &str = "delete";
    pub const ESCAPE: &str = "escape";
    pub const ENTER: &str = "enter";
    pub const BACKSPACE: &str = "backspace";
    pub const TAB: &str = "tab";
    pub const SPACE: &str = "space";
    pub const UP: &str = "up";
    pub const DOWN: &str = "down";
    pub const LEFT: &str = "left";
    pub const RIGHT: &str = "right";
    pub const CAPS_LOCK: &str = "capslock";
    pub const NUM_LOCK: &str = "numlock";
    pub const SCROLL_LOCK: &str = "scrolllock";
    pub const PRINT_SCREEN: &str = "printscreen";
    pub const PAUSE: &str = "pause";
    pub const MENU: &str = "menu";
    pub const CANCEL: &str = "cancel";
    pub const CLEAR: &str = "clear";
    pub const SELECT: &str = "select";
    pub const PRINT: &str = "print";
    pub const EXECUTE: &str = "execute";
    pub const HELP: &str = "help";
    pub const SLEEP: &str = "sleep";
    pub const F1: &str = "f1";
    pub const F2: &str = "f2";
    pub const F3: &str = "f3";
    pub const F4: &str = "f4";
    pub const F5: &str = "f5";
    pub const F6: &str = "f6";
    pub const F7: &str = "f7";
    pub const F8: &str = "f8";
    pub const F9: &str = "f9";
    pub const F10: &str = "f10";
    pub const F11: &str = "f11";
    pub const F12: &str = "f12";
    pub const F13: &str = "f13";
    pub const F14: &str = "f14";
    pub const F15: &str = "f15";
    pub const F16: &str = "f16";
    pub const F17: &str = "f17";
    pub const F18: &str = "f18";
    pub const F19: &str = "f19";
    pub const F20: &str = "f20";
    pub const F21: &str = "f21";
    pub const F22: &str = "f22";
    pub const F23: &str = "f23";
    pub const F24: &str = "f24";
    pub const NUMPAD0: &str = "numpad0";
    pub const NUMPAD1: &str = "numpad1";
    pub const NUMPAD2: &str = "numpad2";
    pub const NUMPAD3: &str = "numpad3";
    pub const NUMPAD4: &str = "numpad4";
    pub const NUMPAD5: &str = "numpad5";
    pub const NUMPAD6: &str = "numpad6";
    pub const NUMPAD7: &str = "numpad7";
    pub const NUMPAD8: &str = "numpad8";
    pub const NUMPAD9: &str = "numpad9";
    pub const NUMPAD_ADD: &str = "numpadadd";
    pub const NUMPAD_SUBTRACT: &str = "numpadsubtract";
    pub const NUMPAD_MULTIPLY: &str = "numpadmultiply";
    pub const NUMPAD_DIVIDE: &str = "numpaddivide";
    pub const NUMPAD_DECIMAL: &str = "numpaddecimal";
    pub const NUMPAD_ENTER: &str = "numpadenter";
    pub const NUMPAD_SEPARATOR: &str = "numpadseparator";
    pub const VOLUME_MUTE: &str = "volumemute";
    pub const VOLUME_DOWN: &str = "volumedown";
    pub const VOLUME_UP: &str = "volumeup";
    pub const MEDIA_NEXT_TRACK: &str = "medianexttrack";
    pub const MEDIA_PREV_TRACK: &str = "mediaprevtrack";
    pub const MEDIA_STOP: &str = "mediastop";
    pub const MEDIA_PLAY_PAUSE: &str = "mediaplaypause";
    pub const BROWSER_BACK: &str = "browserback";
    pub const BROWSER_FORWARD: &str = "browserforward";
    pub const BROWSER_REFRESH: &str = "browserrefresh";
    pub const BROWSER_STOP: &str = "browserstop";
    pub const BROWSER_SEARCH: &str = "browsersearch";
    pub const BROWSER_FAVORITES: &str = "browserfavorites";
    pub const BROWSER_HOME: &str = "browserhome";
    pub const LAUNCH_MAIL: &str = "launchmail";
    pub const LAUNCH_MEDIA: &str = "launchmedia";
    pub const LAUNCH_APP1: &str = "launchapp1";
    pub const LAUNCH_APP2: &str = "launchapp2";

    #[cfg(test)]
    pub(crate) const ALL: &[&str] = &[
        PAGE_UP,
        PAGE_DOWN,
        HOME,
        END,
        INSERT,
        DELETE,
        ESCAPE,
        ENTER,
        BACKSPACE,
        TAB,
        SPACE,
        UP,
        DOWN,
        LEFT,
        RIGHT,
        CAPS_LOCK,
        NUM_LOCK,
        SCROLL_LOCK,
        PRINT_SCREEN,
        PAUSE,
        MENU,
        CANCEL,
        CLEAR,
        SELECT,
        PRINT,
        EXECUTE,
        HELP,
        SLEEP,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F11,
        F12,
        F13,
        F14,
        F15,
        F16,
        F17,
        F18,
        F19,
        F20,
        F21,
        F22,
        F23,
        F24,
        NUMPAD0,
        NUMPAD1,
        NUMPAD2,
        NUMPAD3,
        NUMPAD4,
        NUMPAD5,
        NUMPAD6,
        NUMPAD7,
        NUMPAD8,
        NUMPAD9,
        NUMPAD_ADD,
        NUMPAD_SUBTRACT,
        NUMPAD_MULTIPLY,
        NUMPAD_DIVIDE,
        NUMPAD_DECIMAL,
        NUMPAD_ENTER,
        NUMPAD_SEPARATOR,
        VOLUME_MUTE,
        VOLUME_DOWN,
        VOLUME_UP,
        MEDIA_NEXT_TRACK,
        MEDIA_PREV_TRACK,
        MEDIA_STOP,
        MEDIA_PLAY_PAUSE,
        BROWSER_BACK,
        BROWSER_FORWARD,
        BROWSER_REFRESH,
        BROWSER_STOP,
        BROWSER_SEARCH,
        BROWSER_FAVORITES,
        BROWSER_HOME,
        LAUNCH_MAIL,
        LAUNCH_MEDIA,
        LAUNCH_APP1,
        LAUNCH_APP2,
    ];
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub(crate) struct Shortcut {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
    pub keys: HashSet<String>,
}

impl Shortcut {
    pub fn from_string(keybind: String) -> Self {
        let lowercase_keybind = keybind.to_lowercase();
        let keys = lowercase_keybind.split("+");
        let mut shift = false;
        let mut alt = false;
        let mut ctrl = false;
        let mut meta = false;
        let mut chars = HashSet::new();
        keys.for_each(|x| match x {
            "shift" => shift = true,
            "alt" => alt = true,
            "ctrl" => ctrl = true,
            "meta" => meta = true,
            _ => {
                chars.insert(x.to_owned());
            }
        });
        Self {
            shift,
            alt,
            ctrl,
            meta,
            keys: chars,
        }
    }
}

impl ToString for Shortcut {
    fn to_string(&self) -> String {
        let mut res = String::new();
        // formatted for https://specifications.freedesktop.org/shortcuts-spec/latest/#specification
        if self.shift {
            res.push_str("+SHIFT");
        }
        if self.alt {
            res.push_str("+ALT");
        }
        if self.ctrl {
            res.push_str("+CTRL");
        }
        if self.meta {
            res.push_str("+META");
        }
        if !self.keys.is_empty() {
            res.push_str(
                &self
                    .keys
                    .iter()
                    .map(|x| format!("+{}", x))
                    .collect::<String>(),
            );
        }
        res.trim_start_matches("+").to_owned()
    }
}

impl Keybinds {
    pub fn register_keybind(&mut self, keybind: Shortcut, id: KeybindId) {
        self.keybinds.push((keybind, id));
    }
    pub fn clear(&mut self) {
        self.keybinds.clear();
    }
    pub fn get_active_keybinds(&self, keys: &Shortcut) -> Vec<KeybindId> {
        self.keybinds
            .iter()
            .filter(|x| {
                x.0.keys.is_subset(&keys.keys)
                    && (!x.0.alt || (x.0.alt == keys.alt))
                    && (!x.0.ctrl || (x.0.ctrl == keys.ctrl))
                    && (!x.0.shift || (x.0.shift == keys.shift))
                    && (!x.0.meta || (x.0.meta == keys.meta))
            })
            .map(|x| x.1.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressed(keys: &[&str], ctrl: bool, alt: bool, shift: bool, meta: bool) -> Shortcut {
        Shortcut {
            ctrl,
            alt,
            shift,
            meta,
            keys: keys.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn from_string_lowercases_and_splits_modifiers() {
        let s = Shortcut::from_string("Ctrl+Shift+PageUp".to_owned());
        assert!(s.ctrl && s.shift && !s.alt && !s.meta);
        let expected: HashSet<String> = ["pageup"].iter().map(|s| s.to_string()).collect();
        assert_eq!(s.keys, expected);
    }

    #[test]
    fn named_key_registration_matches_pressed_token() {
        // A consumer registers "ctrl+pageup"; the press side reports the canonical
        // token "pageup" (see `tokens` + `*_to_token`). They must match.
        let mut kb = Keybinds::default();
        kb.register_keybind(
            Shortcut::from_string("ctrl+pageup".to_owned()),
            "id".to_owned(),
        );
        let down = pressed(&[tokens::PAGE_UP], true, false, false, false);
        assert_eq!(kb.get_active_keybinds(&down), vec!["id".to_string()]);
    }

    #[test]
    fn regression_case_sensitivity() {
        // The old bug: `from_string` lowercases the registration, but the press
        // side inserted a capitalized name (`GetKeyNameTextW` "Space" / keysym
        // "Return"), so the case-sensitive subset match never fired. Canonical
        // tokens are lowercase, so a lowercase press matches; a capitalized one
        // still must not (documents the exact failure mode this fix removes).
        let mut kb = Keybinds::default();
        kb.register_keybind(Shortcut::from_string("space".to_owned()), "id".to_owned());
        let lower = pressed(&["space"], false, false, false, false);
        let upper = pressed(&["Space"], false, false, false, false);
        assert_eq!(kb.get_active_keybinds(&lower), vec!["id".to_string()]);
        assert!(kb.get_active_keybinds(&upper).is_empty());
    }

    #[test]
    fn tokens_are_lowercase_and_unique() {
        let unique: HashSet<&&str> = tokens::ALL.iter().collect();
        assert_eq!(unique.len(), tokens::ALL.len(), "duplicate token");
        for t in tokens::ALL {
            assert_eq!(*t, t.to_lowercase(), "token not lowercase: {t}");
            assert!(
                !t.is_empty() && !t.contains(' ') && !t.contains('+'),
                "token contains a separator/space or is empty: {t:?}"
            );
        }
    }
}
