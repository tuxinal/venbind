use std::collections::HashMap;

pub type KeybindId = u32;

#[derive(Default)]
pub struct Keybinds {
    keybinds: HashMap<Keybind, KeybindId>,
}

pub enum KeybindTrigger {
    Pressed(KeybindId),
    Released(KeybindId)
}

#[derive(PartialEq, Eq, Hash, Debug)]
pub(crate) struct Keybind {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub character: Option<String>,
}

impl Keybind {
    pub fn from_string(keybind: String) -> Self {
        let keys = keybind.split("+");
        let mut shift = false;
        let mut alt = false;
        let mut ctrl = false;
        let mut character = None;
        keys.for_each(|x| match x {
            "shift" => shift = true,
            "alt" => alt = true,
            "ctrl" => ctrl = true,
            _ => character = Some(x.to_owned())
        });
        Self {
            shift,
            alt,
            ctrl,
            character,
        }
    }
}

impl Keybinds {
    pub fn register_keybind(&mut self, keybind: Keybind, id: KeybindId) {
        self.keybinds.insert(keybind, id);
    }
    pub fn unregister_keybind(&mut self, id: KeybindId) {
        self.keybinds.retain(|_, x| *x != id);
    }
    pub fn get_keybind_id(&self, keybind: Keybind) -> Option<KeybindId> {
        self.keybinds.get(&keybind).copied()
    }
}
