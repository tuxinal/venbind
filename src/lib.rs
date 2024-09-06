mod errors;
#[cfg(all(feature = "node", not(test)))]
pub mod js; // BREAKS TESTS
mod structs;
mod utils;

#[cfg_attr(target_os = "linux", path = "linux.rs")]
mod platform;

use std::sync::mpsc::Sender;

use platform::*;
use structs::{KeybindId, KeybindTrigger};

pub fn start_keybinds(window_id: Option<usize>, display_id: Option<usize>, tx: Sender<KeybindTrigger>) {
    start_keybinds_internal(window_id, display_id, tx).unwrap();
}

pub fn register_keybind(keybind: String, id: KeybindId) {
    register_keybind_internal(keybind, id).unwrap();
}
pub fn unregister_keybind(id: KeybindId) {
    unregister_keybind_internal(id).unwrap();
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc::channel, thread};

    use crate::{register_keybind, start_keybinds, structs::KeybindTrigger};
    #[test]
    fn demo() {
        let (tx, rx) = channel::<KeybindTrigger>();
        thread::spawn(|| {
            start_keybinds(None, None, tx);
        });
        thread::sleep(std::time::Duration::from_secs(1));
        register_keybind("shift+alt+m".to_string(), 1);
        register_keybind("shift+ctrl+a".to_string(), 2);
        loop {
            match rx.recv() {
                Err(err) => {
                    panic!("{err}");
                }
                Ok(KeybindTrigger::Pressed(x)) => {
                    println!("pressed {}", x);
                }
                Ok(KeybindTrigger::Released(x)) => {
                    println!("released {}", x);
                }
            }
        }
    }
}
