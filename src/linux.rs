use ashpd::desktop::*;
use ashpd::zbus::export::futures_util::StreamExt;
use ashpd::WindowIdentifier;
use core::panic;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::Sender, Mutex};
use std::sync::{LazyLock, OnceLock};
use uiohook_sys::{
    _event_type_EVENT_KEY_PRESSED, _uiohook_event, hook_run, hook_set_dispatch_proc,
    UIOHOOK_SUCCESS,
};
use xcb::Extension;
use xkbcommon::xkb::{self, State};

use crate::errors::{Result, VenbindError};
use crate::structs::{Keybind, KeybindId, KeybindTrigger, Keybinds};
use crate::utils;

static KEYBINDS: LazyLock<Mutex<Keybinds>> = LazyLock::new(|| Mutex::new(Keybinds::default()));
static TX: OnceLock<Sender<KeybindTrigger>> = OnceLock::new();

static IS_USING_PORTAL: AtomicBool = AtomicBool::new(true);
static XDG_RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());
static XDG_STATE: LazyLock<tokio::sync::Mutex<Option<XDGState>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

thread_local! {
    static XKBCOMMON_STATE: RefCell<Option<State>> = RefCell::new(None);
}

struct XDGState<'a> {
    portal: global_shortcuts::GlobalShortcuts<'a>,
    session: Session<'a, ashpd::desktop::global_shortcuts::GlobalShortcuts<'a>>,
    window_handle: WindowIdentifier,
}

// window_id should be either a XID if using an X server or a wayland surface handle
// display_id should be a wayland display handle, or None if using X
pub(crate) fn start_keybinds_internal(
    window_id: Option<u64>,
    display_id: Option<u64>,
    tx: Sender<KeybindTrigger>,
) -> Result<()> {
    TX.set(tx).unwrap();
    let result = if utils::is_wayland() || utils::use_xdg_on_x11() {
        Some(xdg_start_keybinds(window_id, display_id))
    } else {
        None
    };
    if let Some(result) = result {
        match result {
            Ok(_) => Ok(()),
            Err(e) => {
                eprintln!("Failed to start using XDG Desktop Portals: {}", e);
                IS_USING_PORTAL.store(false, Ordering::Relaxed);
                xcb_start_keybinds()
            }
        }
    } else {
        IS_USING_PORTAL.store(false, Ordering::Relaxed);
        xcb_start_keybinds()
    }
}

pub(crate) fn register_keybind_internal(keybind: String, id: KeybindId) -> Result<()> {
    if IS_USING_PORTAL.load(Ordering::Relaxed) {
        xdg_register_keybind(keybind, id)
    } else {
        xcb_register_keybind(keybind, id)
    }
}

pub(crate) fn unregister_keybind_internal(id: KeybindId) -> Result<()> {
    if IS_USING_PORTAL.load(Ordering::Relaxed) {
        Err(VenbindError::Message("todo".to_owned()))
    } else {
        xcb_unregister_keybind(id)
    }
}

#[inline]
fn convert_to_pointer<T>(value: Option<u64>) -> *mut T {
    assert_eq!(size_of::<usize>(), size_of::<*mut T>());
    match value {
        Some(value) => value as *mut T,
        None => std::ptr::null_mut(),
    }
}

fn xdg_start_keybinds(window_id: Option<u64>, display_id: Option<u64>) -> Result<()> {
    use ashpd::desktop::global_shortcuts::*;

    if window_id.is_none() {
        eprintln!("Window ID is not valid trying to create portal anyway");
    }

    let res = XDG_RUNTIME.block_on(async {
        let portal = GlobalShortcuts::new().await?;

        let session = portal.create_session().await?;

        let window_handle = if utils::is_wayland() {
            if window_id.is_none() || display_id.is_none() {
                WindowIdentifier::default();
            }
            unsafe {
                WindowIdentifier::from_wayland_raw(
                    convert_to_pointer(window_id),
                    convert_to_pointer(display_id),
                )
                .await
            }
        } else {
            if window_id.is_none() {
                WindowIdentifier::from_xid(window_id.unwrap() as _)
            } else {
                WindowIdentifier::default()
            }
        };

        Ok((portal, session, window_handle))
    });

    match res {
        Ok((portal, session, window_handle)) => {
            let mut state = XDG_STATE.blocking_lock();
            let _ = state.replace(XDGState {
                portal,
                session,
                window_handle,
            });
            XDG_RUNTIME.spawn(xdg_input_thread());
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

async fn xdg_input_thread() {
    let (mut activated, mut deactivted) = {
        let state = XDG_STATE.lock().await;
        if let Some(state) = state.as_ref() {
            let activated = state.portal.receive_activated().await.unwrap();
            let deactivated = state.portal.receive_deactivated().await.unwrap();
            (activated, deactivated)
        } else {
            panic!("This Thread should not be active no XDG state");
        }
    };
    loop {
        while let Some(action) = activated.next().await {
            let local = action.shortcut_id().to_string();
            TX.get()
                .unwrap()
                .send(KeybindTrigger::Pressed(local.parse().unwrap()))
                .unwrap()
        }

        while let Some(action) = deactivted.next().await {
            let local = action.shortcut_id().to_string();
            TX.get()
                .unwrap()
                .send(KeybindTrigger::Released(local.parse().unwrap()))
                .unwrap()
        }
    }
}

fn generic_register_keybind(keybind: String, id: KeybindId) {
    let mut keybinds = KEYBINDS.lock().unwrap();
    keybinds.register_keybind(Keybind::from_string(keybind.clone()), id);
}

fn xdg_register_keybind(keybind: String, id: KeybindId) -> Result<()> {
    use global_shortcuts::NewShortcut;
    let shortcut = NewShortcut::new(format!("{}", id), id.to_string())
        .preferred_trigger(Some(keybind.clone().as_str()));
    let request = XDG_RUNTIME.block_on(async move {
        let lock = XDG_STATE.lock().await;
        let state = lock.as_ref().unwrap();

        let res = state
            .portal
            .bind_shortcuts(&state.session, &[shortcut], &state.window_handle)
            .await;
        generic_register_keybind(keybind, id);
        res
    })?;
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn xcb_dispatch_proc(event_ref: *mut _uiohook_event) {
    let event = &*event_ref;
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

fn xcb_start_keybinds() -> Result<()> {
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
        hook_set_dispatch_proc(Some(xcb_dispatch_proc));
        if hook_run() != UIOHOOK_SUCCESS as i32 {
            return Err(VenbindError::LibUIOHookError);
        }
    };
    Ok(())
}

fn xcb_register_keybind(keybind: String, id: KeybindId) -> Result<()> {
    generic_register_keybind(keybind, id);
    Ok(())
}

fn xcb_unregister_keybind(id: KeybindId) -> Result<()> {
    let mut keybinds = KEYBINDS.lock().unwrap();
    keybinds.unregister_keybind(id);
    Ok(())
}
