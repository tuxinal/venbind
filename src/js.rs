use std::{sync::mpsc::channel, thread};

use napi::{
    bindgen_prelude::*,
    threadsafe_function::{ErrorStrategy, ThreadsafeFunction, ThreadsafeFunctionCallMode},
};
use napi_derive::napi;

use crate::structs::{KeybindId, KeybindTrigger};

#[napi(ts_args_type = "callback: (err: null | Error, id: number) => void")]
pub fn start_keybinds(callback: JsFunction) -> Result<()> {
    let (tx, rx) = channel::<KeybindTrigger>();
    thread::spawn(|| {
        crate::start_keybinds(tx);
    });

    let thread_function: ThreadsafeFunction<u32, ErrorStrategy::Fatal> = callback
        .create_threadsafe_function(0, |ctx| ctx.env.create_uint32(ctx.value).map(|v| vec![v]))?;
    thread::spawn(move || loop {
        match rx.recv() {
            Err(err) => {
                panic!("{err}");
            }
            Ok(KeybindTrigger::Pressed(x)) => {
                thread_function.call(x, ThreadsafeFunctionCallMode::Blocking);
            }
            Ok(KeybindTrigger::Released(x)) => {
                println!("released {}", x);
            }
        }
    });

    Ok(())
}

#[napi]
pub fn register_keybind(keybind: String, #[napi(ts_arg_type = "number")] id: KeybindId) {
    crate::register_keybind(keybind, id);
}

#[napi]
pub fn unregister_keybind(#[napi(ts_arg_type = "number")] id: KeybindId) {
    crate::unregister_keybind(id);
}
