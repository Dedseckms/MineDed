//! Push-to-talk: polls the global key state (Windows `GetAsyncKeyState`, X11
//! `XQueryKeymap`) on a small thread. Polling never swallows the key, so the
//! game still receives it. Wayland has no global key access → caller falls
//! back to VAD.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use device_query::{DeviceQuery, DeviceState, Keycode};

const POLL: Duration = Duration::from_millis(15);

/// Why PTT cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PttError {
    NoKey,
    UnknownKey(String),
    Wayland,
    Unavailable,
}

impl PttError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NoKey => "noKey",
            Self::UnknownKey(_) => "unknownKey",
            Self::Wayland => "wayland",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Maps a `keyboard-types` code name (`KeyV`, `CapsLock`, `F5`…) to a key.
pub fn parse_key(code: &str) -> Option<Keycode> {
    use Keycode as K;
    if let Some(c) = code.strip_prefix("Key") {
        let mut chars = c.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            return letter(ch);
        }
        return None;
    }
    if let Some(d) = code.strip_prefix("Digit") {
        return digit(d, false);
    }
    if let Some(n) = code.strip_prefix("Numpad") {
        if let Some(k) = digit(n, true) {
            return Some(k);
        }
    }
    if let Some(n) = code.strip_prefix('F') {
        if let Ok(n) = n.parse::<u8>() {
            return function(n);
        }
    }
    Some(match code {
        "Space" => K::Space,
        "Escape" => K::Escape,
        "Enter" => K::Enter,
        "Tab" => K::Tab,
        "Backspace" => K::Backspace,
        "CapsLock" => K::CapsLock,
        "ShiftLeft" => K::LShift,
        "ShiftRight" => K::RShift,
        "ControlLeft" => K::LControl,
        "ControlRight" => K::RControl,
        "AltLeft" => K::LAlt,
        "AltRight" => K::RAlt,
        "MetaLeft" => K::LMeta,
        "MetaRight" => K::RMeta,
        "ArrowUp" => K::Up,
        "ArrowDown" => K::Down,
        "ArrowLeft" => K::Left,
        "ArrowRight" => K::Right,
        "Home" => K::Home,
        "End" => K::End,
        "PageUp" => K::PageUp,
        "PageDown" => K::PageDown,
        "Insert" => K::Insert,
        "Delete" => K::Delete,
        "NumpadAdd" => K::NumpadAdd,
        "NumpadSubtract" => K::NumpadSubtract,
        "NumpadMultiply" => K::NumpadMultiply,
        "NumpadDivide" => K::NumpadDivide,
        "NumpadDecimal" => K::NumpadDecimal,
        "NumpadEnter" => K::NumpadEnter,
        "NumpadEqual" => K::NumpadEquals,
        "Backquote" => K::Grave,
        "Minus" => K::Minus,
        "Equal" => K::Equal,
        "BracketLeft" => K::LeftBracket,
        "BracketRight" => K::RightBracket,
        "Backslash" => K::BackSlash,
        "Semicolon" => K::Semicolon,
        "Quote" => K::Apostrophe,
        "Comma" => K::Comma,
        "Period" => K::Dot,
        "Slash" => K::Slash,
        _ => return None,
    })
}

fn letter(ch: char) -> Option<Keycode> {
    use Keycode as K;
    const LETTERS: [Keycode; 26] = [
        K::A,
        K::B,
        K::C,
        K::D,
        K::E,
        K::F,
        K::G,
        K::H,
        K::I,
        K::J,
        K::K,
        K::L,
        K::M,
        K::N,
        K::O,
        K::P,
        K::Q,
        K::R,
        K::S,
        K::T,
        K::U,
        K::V,
        K::W,
        K::X,
        K::Y,
        K::Z,
    ];
    if ch.is_ascii_uppercase() {
        LETTERS.get((ch as u8 - b'A') as usize).copied()
    } else {
        None
    }
}

fn digit(d: &str, numpad: bool) -> Option<Keycode> {
    use Keycode as K;
    const DIGITS: [Keycode; 10] = [
        K::Key0,
        K::Key1,
        K::Key2,
        K::Key3,
        K::Key4,
        K::Key5,
        K::Key6,
        K::Key7,
        K::Key8,
        K::Key9,
    ];
    const NUMPAD: [Keycode; 10] = [
        K::Numpad0,
        K::Numpad1,
        K::Numpad2,
        K::Numpad3,
        K::Numpad4,
        K::Numpad5,
        K::Numpad6,
        K::Numpad7,
        K::Numpad8,
        K::Numpad9,
    ];
    if d.len() != 1 {
        return None;
    }
    let n = d.parse::<usize>().ok()?;
    if numpad {
        NUMPAD.get(n).copied()
    } else {
        DIGITS.get(n).copied()
    }
}

fn function(n: u8) -> Option<Keycode> {
    use Keycode as K;
    const FS: [Keycode; 20] = [
        K::F1,
        K::F2,
        K::F3,
        K::F4,
        K::F5,
        K::F6,
        K::F7,
        K::F8,
        K::F9,
        K::F10,
        K::F11,
        K::F12,
        K::F13,
        K::F14,
        K::F15,
        K::F16,
        K::F17,
        K::F18,
        K::F19,
        K::F20,
    ];
    FS.get(usize::from(n).checked_sub(1)?).copied()
}

/// Wayland sessions: no global key state.
pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"))
            || std::env::var_os("WAYLAND_DISPLAY").is_some())
}

/// Running key poller. Dropping it stops the thread.
pub struct PttWatcher {
    pressed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl PttWatcher {
    /// Starts polling `code`. Fails on Wayland, unknown keys or no X display.
    pub fn start(code: Option<&str>) -> Result<Self, PttError> {
        let code = code.ok_or(PttError::NoKey)?;
        let key = parse_key(code).ok_or_else(|| PttError::UnknownKey(code.to_owned()))?;
        if is_wayland() {
            return Err(PttError::Wayland);
        }
        let pressed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (init_tx, init_rx) = mpsc::sync_channel::<bool>(1);
        let (p, s) = (pressed.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("voice-ptt".into())
            .spawn(move || {
                // `DeviceState` holds an X connection (not `Send`): create it here.
                let Some(state) = DeviceState::checked_new() else {
                    let _ = init_tx.send(false);
                    return;
                };
                let _ = init_tx.send(true);
                while !s.load(Ordering::Relaxed) {
                    let down = state.get_keys().contains(&key);
                    p.store(down, Ordering::Relaxed);
                    std::thread::sleep(POLL);
                }
                p.store(false, Ordering::Relaxed);
            })
            .map_err(|_| PttError::Unavailable)?;
        match init_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(true) => Ok(Self {
                pressed,
                stop,
                thread: Some(thread),
            }),
            _ => {
                stop.store(true, Ordering::Relaxed);
                Err(PttError::Unavailable)
            }
        }
    }

    pub fn pressed(&self) -> bool {
        self.pressed.load(Ordering::Relaxed)
    }
}

impl Drop for PttWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        assert_eq!(parse_key("KeyV"), Some(Keycode::V));
        assert_eq!(parse_key("Digit7"), Some(Keycode::Key7));
        assert_eq!(parse_key("Numpad3"), Some(Keycode::Numpad3));
        assert_eq!(parse_key("NumpadEnter"), Some(Keycode::NumpadEnter));
        assert_eq!(parse_key("F12"), Some(Keycode::F12));
        assert_eq!(parse_key("CapsLock"), Some(Keycode::CapsLock));
        assert_eq!(parse_key("AltLeft"), Some(Keycode::LAlt));
        assert_eq!(parse_key("Backquote"), Some(Keycode::Grave));
        assert_eq!(parse_key("Keyv"), None);
        assert_eq!(parse_key("F0"), None);
        assert_eq!(parse_key("Mouse4"), None);
        assert_eq!(PttWatcher::start(None).err(), Some(PttError::NoKey));
        assert!(matches!(
            PttWatcher::start(Some("Nope")).err(),
            Some(PttError::UnknownKey(_))
        ));
    }
}
