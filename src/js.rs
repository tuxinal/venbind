use std::{sync::mpsc::channel, thread};

use node_bindgen::derive::node_bindgen;

use crate::structs::{KeybindId, KeybindTrigger};

#[node_bindgen]
async fn start_keybinds<F: Fn(KeybindId)>(
    window_id: Option<u64>,
    display_id: Option<u64>,
    callback: F,
) {
    let (tx, rx) = channel::<KeybindTrigger>();
    thread::spawn(|| {
        crate::start_keybinds(window_id, display_id, tx);
    });
    loop {
        match rx.recv() {
            Err(err) => {
                panic!("{err}");
            }
            Ok(KeybindTrigger::Pressed(x)) => {
                callback(x);
            }
            Ok(KeybindTrigger::Released(x)) => {
                println!("released {}", x);
            }
        }
    }
}
#[node_bindgen]
pub fn register_keybind(keybind: String, id: KeybindId) {
    crate::register_keybind(keybind, id);
}
#[node_bindgen]
pub fn unregister_keybind(id: KeybindId) {
    crate::unregister_keybind(id);
}
