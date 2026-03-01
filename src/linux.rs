use ashpd::{
    desktop::{global_shortcuts::*, *},
    register_host_app,
    zbus::export::futures_util::StreamExt,
    AppID,
};
use futures::{executor::block_on, future::Either};
use std::{
    cell::RefCell,
    collections::HashSet,
    env,
    str::FromStr,
    sync::{mpsc::Sender, LazyLock, Mutex, OnceLock},
};
use uiohook_sys::{
    _event_type_EVENT_KEY_PRESSED, _event_type_EVENT_KEY_RELEASED, _uiohook_event, hook_run,
    hook_set_dispatch_proc, UIOHOOK_SUCCESS,
};
use xcb::Extension;
use xkbcommon::xkb::{self, Keysym, State};

use crate::structs::{KeybindInfo, KeybindTrigger, Keybinds, Shortcut};
use crate::{
    errors::{Result, VenbindError},
    structs::KeybindId,
};

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

static XDG_STATE: LazyLock<Mutex<Option<XDGState>>> = LazyLock::new(|| Mutex::new(None));

thread_local! {
    static XKBCOMMON_STATE: RefCell<Option<State>> = RefCell::new(None);
}

struct XDGState<'a> {
    portal: global_shortcuts::GlobalShortcuts<'a>,
    session: Session<'a, ashpd::desktop::global_shortcuts::GlobalShortcuts<'a>>,
}

pub(crate) fn start_keybinds_internal(
    tx: Sender<KeybindTrigger>,
    app_id: Option<String>,
) -> Result<()> {
    TX.set(tx).unwrap();
    if using_xdg() {
        block_on(xdg_start_keybinds(app_id))
    } else {
        uiohook_start_keybinds()
    }
}

pub(crate) fn set_keybinds_internal(keybinds: Vec<KeybindInfo>) -> Result<()> {
    if using_xdg() {
        xdg_set_keybinds(keybinds)
    } else {
        uiohook_set_keybinds(keybinds)
    }
}

async fn xdg_start_keybinds(app_id: Option<String>) -> Result<()> {
    if let Some(app_id) = app_id {
        if let Err(err) = register_host_app(AppID::from_str(&app_id)?).await {
            eprintln!("Couldn't use registry (chances are your version of xdg-desktop-portal is old): {err}")
        }
    }
    let mut state = XDG_STATE.lock().unwrap();
    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session().await?;

    state.replace(XDGState { portal, session });
    drop(state);

    xdg_input_thread().await?;

    Ok(())
}

async fn xdg_input_thread() -> Result<()> {
    let (mut activated, mut deactivted) = {
        let state = XDG_STATE.lock().unwrap();
        if let Some(state) = state.as_ref() {
            let activated = state.portal.receive_activated().await?;
            let deactivated = state.portal.receive_deactivated().await?;
            (activated, deactivated)
        } else {
            panic!("This Thread should not be active no XDG state");
        }
    };
    loop {
        match futures::future::select(activated.next(), deactivted.next()).await {
            Either::Left((Some(activated), _)) => TX
                .get()
                .unwrap()
                .send(KeybindTrigger::Pressed(activated.shortcut_id().to_owned()))?,
            Either::Right((Some(deactivated), _)) => {
                let id = deactivated.shortcut_id().to_owned();
                let keybinds = KEYBINDS.lock().unwrap();
                if !keybinds
                    .get_active_keybinds(&CURR_DOWN.lock().unwrap())
                    .into_iter()
                    .any(|x| x == id)
                {
                    // Otherwise, release
                    TX.get().unwrap().send(KeybindTrigger::Released(id))?
                }
            }
            _ => {
                eprintln!("Unexpected output from GlobalShortcuts!");
            }
        }
    }
}

fn xdg_set_keybinds(keybinds: Vec<KeybindInfo>) -> Result<()> {
    if !using_xdg() {
        return Err(VenbindError::UnsupportedOnXdg);
    }
    let shortcuts: Vec<NewShortcut> = keybinds
        .iter()
        .map(|x| NewShortcut::new(&x.id, x.name.clone().unwrap_or(x.id.clone())))
        .collect();
    let lock = XDG_STATE.lock().unwrap();
    if let Some(state) = lock.as_ref() {
        let listshortcuts = block_on(state.portal.list_shortcuts(&state.session))?.response()?;
        let curr_shortcuts = listshortcuts.shortcuts();

        if !keybinds
            .iter()
            .all(|x| curr_shortcuts.iter().any(|y| y.id() == x.id))
        {
            block_on(
                state
                    .portal
                    .bind_shortcuts(&state.session, &shortcuts, None),
            )?;
        }
    } else {
        eprintln!("No GlobalShortcuts state was found! skipping preregistery.");
    }
    Ok(())
}

#[no_mangle]
pub extern "C" fn uiohook_dispatch_proc(event_ref: *mut _uiohook_event) {
    let event = &unsafe { *event_ref };
    if event.type_ == _event_type_EVENT_KEY_PRESSED || event.type_ == _event_type_EVENT_KEY_RELEASED
    {
        XKBCOMMON_STATE.with(|state| {
            let state_borrow = state.borrow();
            let state = state_borrow.as_ref().unwrap();
            let shift = event.mask & uiohook_sys::MASK_SHIFT as u16 != 0;
            let alt = event.mask & uiohook_sys::MASK_ALT as u16 != 0;
            let ctrl = event.mask & uiohook_sys::MASK_CTRL as u16 != 0;
            let meta = event.mask & uiohook_sys::MASK_META as u16 != 0;
            let keycode =
                unsafe { uiohook_sys::platform::scancode_to_keycode(event.data.keyboard.keycode) };
            // get the keysym from the keycode to always use a static keyboard layout
            let keysym = state.key_get_one_sym(keycode.into());
            let key = match keysym {
                // Keys that do have an ascii representation but the keysym name is more fitting
                Keysym::Escape
                | Keysym::BackSpace
                | Keysym::Return
                | Keysym::Tab
                | Keysym::Delete
                | Keysym::space => {
                    Some(format!("{:?}", keysym).trim_start_matches("XK_").to_owned())
                }
                // Keys that are already considered in the event.mask
                Keysym::Shift_L
                | Keysym::Shift_R
                | Keysym::Control_L
                | Keysym::Control_R
                | Keysym::Alt_L
                | Keysym::Alt_R
                | Keysym::Super_L
                | Keysym::Super_R => None,
                // Everything else
                _ => {
                    let key = state.key_get_utf8(keycode.into());
                    if key.is_empty() {
                        Some(format!("{:?}", keysym).trim_start_matches("XK_").to_owned())
                    } else {
                        Some(key)
                    }
                }
            };
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
        });
    }
}

fn uiohook_start_keybinds() -> Result<()> {
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
        hook_set_dispatch_proc(Some(uiohook_dispatch_proc));
        if hook_run() != UIOHOOK_SUCCESS as i32 {
            return Err(VenbindError::LibUIOHookError);
        }
    };
    Ok(())
}

fn uiohook_set_keybinds(keybinds: Vec<KeybindInfo>) -> Result<()> {
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

#[inline]
pub(crate) fn using_xdg() -> bool {
    env::var("XDG_SESSION_TYPE").is_ok_and(|x| x == "wayland".to_owned())
        || env::var("WAYLAND_DISPLAY").is_ok()
        || env::var("VENBIND_USE_XDG_PORTAL").is_ok()
}
