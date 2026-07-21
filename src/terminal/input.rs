use crate::cell::CellCoord;
use crate::protocol::{Key, Modifiers, MouseButton, MouseKind};

use super::screen::{MouseTrackingMode, TerminalModes};

/// Encode one normalized key press for the child terminal.
///
/// Lock/media/unknown keys return `None`. Application-keypad mode is
/// intentionally not applied to `Key::Char` digits because the normalized
/// protocol cannot distinguish number-row and keypad input.
#[must_use]
pub fn encode_key(key: Key, mods: Modifiers, modes: TerminalModes) -> Option<Vec<u8>> {
    if mods.contains(Modifiers::META) || mods.contains(Modifiers::HYPER) {
        return None;
    }
    let alt = mods.contains(Modifiers::ALT);
    let ctrl = mods.contains(Modifiers::CTRL);
    let mut out = match key {
        Key::Char(ch) => {
            let mut bytes = Vec::with_capacity(4);
            if ctrl {
                bytes.push(control_byte(ch)?);
            } else {
                let mut encoded = [0; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
            }
            bytes
        }
        Key::Enter => vec![b'\r'],
        Key::Tab => vec![b'\t'],
        Key::Backspace => vec![0x7f],
        Key::Escape => vec![0x1b],
        Key::BackTab if mods == Modifiers::NONE || mods == Modifiers::SHIFT => b"\x1b[Z".to_vec(),
        Key::BackTab => modified_csi(b'Z', mods, None),
        Key::Up => navigation(b'A', mods, modes.application_cursor),
        Key::Down => navigation(b'B', mods, modes.application_cursor),
        Key::Right => navigation(b'C', mods, modes.application_cursor),
        Key::Left => navigation(b'D', mods, modes.application_cursor),
        Key::Home => navigation(b'H', mods, modes.application_cursor),
        Key::End => navigation(b'F', mods, modes.application_cursor),
        Key::Insert => tilde_key(2, mods),
        Key::Delete => tilde_key(3, mods),
        Key::PageUp => tilde_key(5, mods),
        Key::PageDown => tilde_key(6, mods),
        Key::F(n @ 1..=4) => function_1_to_4(n, mods),
        Key::F(n @ 5..=12) => {
            let code = [15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)];
            tilde_key(code, mods)
        }
        Key::Null if ctrl => vec![0],
        Key::F(_)
        | Key::CapsLock
        | Key::ScrollLock
        | Key::NumLock
        | Key::PrintScreen
        | Key::Pause
        | Key::Menu
        | Key::KeypadBegin
        | Key::Null
        | Key::Unknown(_) => return None,
    };
    // Character/control/basic keys use the traditional ESC prefix for Alt.
    // Named CSI keys encode Alt in their xterm modifier parameter already.
    if alt
        && matches!(
            key,
            Key::Char(_) | Key::Enter | Key::Tab | Key::Backspace | Key::Escape
        )
    {
        out.insert(0, 0x1b);
    }
    Some(out)
}

/// Encode pasted bytes, optionally framing them with bracketed-paste markers.
#[must_use]
pub fn encode_paste(bytes: &[u8], bracketed_paste: bool) -> Vec<u8> {
    if !bracketed_paste {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len() + 12);
    out.extend_from_slice(b"\x1b[200~");
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\x1b[201~");
    out
}

/// Encode a focus transition when focus reporting is enabled.
#[must_use]
pub fn encode_focus(focused: bool, focus_reporting: bool) -> Option<Vec<u8>> {
    focus_reporting.then(|| {
        if focused {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        }
    })
}

/// Encode an xterm SGR mouse report using zero-based terminal coordinates.
#[must_use]
pub fn encode_mouse(
    kind: MouseKind,
    coord: CellCoord,
    mods: Modifiers,
    modes: TerminalModes,
) -> Option<Vec<u8>> {
    if mods.contains(Modifiers::META) || mods.contains(Modifiers::HYPER) {
        return None;
    }
    if !modes.mouse_sgr || modes.mouse_tracking == MouseTrackingMode::Off {
        return None;
    }
    let allowed = match modes.mouse_tracking {
        MouseTrackingMode::Off => false,
        MouseTrackingMode::X10 => matches!(kind, MouseKind::Down(_)),
        MouseTrackingMode::Button => !matches!(kind, MouseKind::Move),
        MouseTrackingMode::Any => true,
    };
    if !allowed {
        return None;
    }
    let (mut code, release) = match kind {
        MouseKind::Down(button) => (button_code(button), false),
        MouseKind::Up(_) => (3, true),
        MouseKind::Drag(button) => (button_code(button) + 32, false),
        MouseKind::Move => (35, false),
        MouseKind::ScrollUp => (64, false),
        MouseKind::ScrollDown => (65, false),
        MouseKind::ScrollLeft => (66, false),
        MouseKind::ScrollRight => (67, false),
    };
    if mods.contains(Modifiers::SHIFT) {
        code += 4;
    }
    if mods.contains(Modifiers::ALT) {
        code += 8;
    }
    if mods.contains(Modifiers::CTRL) {
        code += 16;
    }
    let final_byte = if release { 'm' } else { 'M' };
    Some(
        format!(
            "\x1b[<{code};{};{}{final_byte}",
            coord.col.saturating_add(1),
            coord.row.saturating_add(1)
        )
        .into_bytes(),
    )
}

fn control_byte(ch: char) -> Option<u8> {
    match ch {
        '@' | ' ' | '`' => Some(0),
        'a'..='z' => Some(ch as u8 - b'a' + 1),
        'A'..='Z' => Some(ch as u8 - b'A' + 1),
        '[' | '{' => Some(0x1b),
        '\\' | '|' => Some(0x1c),
        ']' | '}' => Some(0x1d),
        '^' | '~' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

fn navigation(final_byte: u8, mods: Modifiers, application: bool) -> Vec<u8> {
    let parameter = modifier_parameter(mods);
    if parameter == 1 {
        vec![0x1b, if application { b'O' } else { b'[' }, final_byte]
    } else {
        modified_csi(final_byte, mods, None)
    }
}

fn function_1_to_4(n: u8, mods: Modifiers) -> Vec<u8> {
    let final_byte = b'P' + n - 1;
    if modifier_parameter(mods) == 1 {
        vec![0x1b, b'O', final_byte]
    } else {
        modified_csi(final_byte, mods, None)
    }
}

fn tilde_key(code: u8, mods: Modifiers) -> Vec<u8> {
    let modifier = modifier_parameter(mods);
    if modifier == 1 {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{modifier}~").into_bytes()
    }
}

fn modified_csi(final_byte: u8, mods: Modifiers, first: Option<u8>) -> Vec<u8> {
    let modifier = modifier_parameter(mods);
    if modifier == 1 && first.is_none() {
        return vec![0x1b, b'[', final_byte];
    }
    let first = first.unwrap_or(1);
    format!("\x1b[{first};{modifier}{}", final_byte as char).into_bytes()
}

fn modifier_parameter(mods: Modifiers) -> u8 {
    1 + u8::from(mods.contains(Modifiers::SHIFT))
        + 2 * u8::from(mods.contains(Modifiers::ALT))
        + 4 * u8::from(mods.contains(Modifiers::CTRL))
}

fn button_code(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes() -> TerminalModes {
        TerminalModes::default()
    }

    #[test]
    fn utf8_ctrl_and_alt_boundaries() {
        assert_eq!(
            encode_key(Key::Char('é'), Modifiers::NONE, modes()),
            Some("é".as_bytes().to_vec())
        );
        assert_eq!(
            encode_key(Key::Char('c'), Modifiers::CTRL, modes()),
            Some(vec![3])
        );
        assert_eq!(
            encode_key(Key::Char('?'), Modifiers::CTRL | Modifiers::ALT, modes()),
            Some(vec![0x1b, 0x7f])
        );
        assert_eq!(encode_key(Key::Char('é'), Modifiers::CTRL, modes()), None);
    }

    #[test]
    fn application_cursor_and_xterm_modifiers() {
        let mut app = modes();
        app.application_cursor = true;
        assert_eq!(
            encode_key(Key::Up, Modifiers::NONE, app),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode_key(Key::Up, Modifiers::CTRL | Modifiers::SHIFT, app),
            Some(b"\x1b[1;6A".to_vec())
        );
        assert_eq!(
            encode_key(Key::Delete, Modifiers::ALT, modes()),
            Some(b"\x1b[3;3~".to_vec())
        );
        assert_eq!(
            encode_key(Key::F(1), Modifiers::NONE, modes()),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            encode_key(Key::F(12), Modifiers::CTRL, modes()),
            Some(b"\x1b[24;5~".to_vec())
        );
        assert_eq!(
            encode_key(Key::BackTab, Modifiers::SHIFT, modes()),
            Some(b"\x1b[Z".to_vec())
        );
    }

    #[test]
    fn ambiguous_digits_ignore_application_keypad() {
        let mut app = modes();
        app.application_keypad = true;
        assert_eq!(
            encode_key(Key::Char('7'), Modifiers::NONE, app),
            Some(b"7".to_vec())
        );
    }

    #[test]
    fn paste_and_focus_are_exact() {
        assert_eq!(encode_paste(b"a\0b", false), b"a\0b".to_vec());
        assert_eq!(
            encode_paste(b"a\0b", true),
            b"\x1b[200~a\0b\x1b[201~".to_vec()
        );
        assert_eq!(encode_focus(true, true), Some(b"\x1b[I".to_vec()));
        assert_eq!(encode_focus(false, true), Some(b"\x1b[O".to_vec()));
        assert_eq!(encode_focus(true, false), None);
    }

    #[test]
    fn sgr_mouse_modes_modifiers_and_coordinates() {
        let mut m = modes();
        m.mouse_sgr = true;
        m.mouse_tracking = MouseTrackingMode::Any;
        assert_eq!(
            encode_mouse(
                MouseKind::Down(MouseButton::Left),
                CellCoord::new(0, 0),
                Modifiers::NONE,
                m
            ),
            Some(b"\x1b[<0;1;1M".to_vec())
        );
        assert_eq!(
            encode_mouse(
                MouseKind::Drag(MouseButton::Right),
                CellCoord::new(511, 511),
                Modifiers::CTRL | Modifiers::ALT,
                m
            ),
            Some(b"\x1b[<58;512;512M".to_vec())
        );
        assert_eq!(
            encode_mouse(
                MouseKind::Up(MouseButton::Left),
                CellCoord::new(4, 9),
                Modifiers::NONE,
                m
            ),
            Some(b"\x1b[<3;10;5m".to_vec())
        );
        assert_eq!(
            encode_mouse(
                MouseKind::ScrollDown,
                CellCoord::new(1, 2),
                Modifiers::SHIFT,
                m
            ),
            Some(b"\x1b[<69;3;2M".to_vec())
        );
    }

    #[test]
    fn unsupported_keys_are_invisible() {
        assert_eq!(encode_key(Key::Unknown(7), Modifiers::NONE, modes()), None);
        assert_eq!(encode_key(Key::F(13), Modifiers::NONE, modes()), None);
        assert_eq!(encode_key(Key::Char('c'), Modifiers::META, modes()), None);
        assert_eq!(encode_key(Key::Up, Modifiers::HYPER, modes()), None);
        let mut mouse_modes = modes();
        mouse_modes.mouse_sgr = true;
        mouse_modes.mouse_tracking = MouseTrackingMode::Any;
        assert_eq!(
            encode_mouse(
                MouseKind::Down(MouseButton::Left),
                CellCoord::new(0, 0),
                Modifiers::META,
                mouse_modes,
            ),
            None,
        );
        assert_eq!(
            encode_mouse(
                MouseKind::Move,
                CellCoord::new(4, 9),
                Modifiers::HYPER,
                mouse_modes,
            ),
            None,
        );
    }
}
