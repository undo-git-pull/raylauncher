//! Global hotkey through Carbon's RegisterEventHotKey. It needs no Accessibility
//! permission and delivers events on the main run loop that NSApplication already runs.

use std::ffi::c_void;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Combo {
    pub keycode: u32,
    pub modifiers: u32, // Carbon modifier mask
}

const CMD: u32 = 1 << 8;
const SHIFT: u32 = 1 << 9;
const OPTION: u32 = 1 << 11;
const CONTROL: u32 = 1 << 12;

fn keycode(name: &str) -> Option<u32> {
    // macOS virtual key codes (ANSI layout positions).
    const LETTERS: [(char, u32); 26] = [
        ('a', 0), ('b', 11), ('c', 8), ('d', 2), ('e', 14), ('f', 3), ('g', 5), ('h', 4), ('i', 34),
        ('j', 38), ('k', 40), ('l', 37), ('m', 46), ('n', 45), ('o', 31), ('p', 35), ('q', 12),
        ('r', 15), ('s', 1), ('t', 17), ('u', 32), ('v', 9), ('w', 13), ('x', 7), ('y', 16), ('z', 6),
    ];
    const DIGITS: [u32; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
    const FKEYS: [u32; 12] = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111];
    let mut ch = name.chars();
    if let (Some(c), None) = (ch.next(), ch.next()) {
        if let Some(&(_, k)) = LETTERS.iter().find(|(l, _)| *l == c) {
            return Some(k);
        }
        if let Some(d) = c.to_digit(10) {
            return Some(DIGITS[d as usize]);
        }
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<usize>().ok()) {
        return FKEYS.get(n.wrapping_sub(1)).copied();
    }
    Some(match name {
        "space" => 49,
        "return" | "enter" => 36,
        "tab" => 48,
        "escape" | "esc" => 53,
        "minus" => 27,
        "equal" => 24,
        "comma" => 43,
        "period" => 47,
        "slash" => 44,
        "semicolon" => 41,
        "quote" => 39,
        "grave" => 50,
        "backslash" => 42,
        "leftbracket" => 33,
        "rightbracket" => 30,
        _ => return None,
    })
}

impl Combo {
    pub fn parse(s: &str) -> Result<Combo, String> {
        let parts: Vec<String> = s.split('+').map(|p| p.trim().to_lowercase()).collect();
        let (key, mods) = parts.split_last().ok_or("empty hotkey")?;
        let mut modifiers = 0;
        for m in mods {
            modifiers |= match m.as_str() {
                "cmd" | "command" | "super" => CMD,
                "alt" | "option" | "opt" => OPTION,
                "ctrl" | "control" => CONTROL,
                "shift" => SHIFT,
                _ => return Err(format!("unknown modifier {m:?} (use cmd, alt, ctrl, shift)")),
            };
        }
        if modifiers == 0 && !key.starts_with('f') {
            return Err(format!("{s:?} has no modifier; add cmd, alt, ctrl or shift"));
        }
        let keycode = keycode(key).ok_or_else(|| format!("unknown key {key:?}"))?;
        Ok(Combo { keycode, modifiers })
    }
}

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}
#[repr(C)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}
type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: Handler,
        count: u32,
        types: *const EventTypeSpec,
        user: *mut c_void,
        out: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        mods: u32,
        id: EventHotKeyID,
        target: *mut c_void,
        options: u32,
        out: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(r: *mut c_void) -> i32;
}

static mut CALLBACK: Option<fn()> = None;

extern "C" fn on_event(_next: *mut c_void, _event: *mut c_void, _user: *mut c_void) -> i32 {
    // SAFETY: CALLBACK is written once on the main thread before any event can arrive.
    if let Some(cb) = unsafe { CALLBACK } {
        cb();
    }
    0
}

/// Install the Carbon event handler. Call once, on the main thread.
pub fn install(cb: fn()) {
    unsafe {
        CALLBACK = Some(cb);
        let spec = EventTypeSpec { class: u32::from_be_bytes(*b"keyb"), kind: 5 }; // kEventHotKeyPressed
        InstallEventHandler(GetApplicationEventTarget(), on_event, 1, &spec, std::ptr::null_mut(), std::ptr::null_mut());
    }
}

pub struct Registration(*mut c_void);

impl Drop for Registration {
    fn drop(&mut self) {
        unsafe { UnregisterEventHotKey(self.0) };
    }
}

pub fn register(c: Combo) -> Result<Registration, String> {
    let mut out = std::ptr::null_mut();
    let id = EventHotKeyID { signature: u32::from_be_bytes(*b"lnch"), id: 1 };
    let status = unsafe { RegisterEventHotKey(c.keycode, c.modifiers, id, GetApplicationEventTarget(), 0, &mut out) };
    if status != 0 || out.is_null() {
        return Err(format!("RegisterEventHotKey failed (status {status}); another app may own this shortcut"));
    }
    Ok(Registration(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse() {
        assert_eq!(Combo::parse("alt+space").unwrap(), Combo { keycode: 49, modifiers: OPTION });
        assert_eq!(Combo::parse("Cmd+Shift+K").unwrap(), Combo { keycode: 40, modifiers: CMD | SHIFT });
        assert_eq!(Combo::parse("f5").unwrap().keycode, 96);
        assert!(Combo::parse("space").is_err());
        assert!(Combo::parse("hyper+k").is_err());
    }
}
