use gpui::Keystroke;
use pmacs_attach_client::AttachClient;
use pmacs_protocol::{Key, Modifiers, TEXT_INPUT_MIN_VERSION, TransportError};

#[derive(Debug, PartialEq)]
pub enum Input<'a> {
    Key(Key, Modifiers),
    Text(&'a str),
}

pub fn translate(stroke: &Keystroke) -> Option<Input<'_>> {
    let m = stroke.modifiers;
    let mut mods = Modifiers::NONE;
    if m.control {
        mods |= Modifiers::CTRL;
    }
    if m.alt {
        mods |= Modifiers::ALT;
    }
    if m.platform {
        mods |= Modifiers::META;
    }
    if m.shift {
        mods |= Modifiers::SHIFT;
    }
    let named = match stroke.key.as_str() {
        "enter" | "return" => Key::Enter,
        "escape" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "tab" if m.shift => Key::BackTab,
        "tab" => Key::Tab,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "insert" => Key::Insert,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "space" => Key::Char(' '),
        key => {
            if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
                && (1..=35).contains(&n)
            {
                return Some(Input::Key(Key::F(n), mods));
            }
            if m.control || m.alt || m.platform {
                let mut chars = key.chars();
                let c = chars.next()?;
                return chars
                    .next()
                    .is_none()
                    .then_some(Input::Key(Key::Char(c), mods));
            }
            let text = stroke.key_char.as_deref()?;
            let mut chars = text.chars();
            let first = chars.next()?;
            if text.chars().any(char::is_control) {
                return None;
            }
            // Shift is already reflected in the layout-resolved text. Single
            // characters go through the daemon's ordinary keymaps, even mid-prefix.
            return Some(if chars.next().is_none() {
                Input::Key(Key::Char(first), Modifiers::NONE)
            } else {
                Input::Text(text)
            });
        }
    };
    Some(Input::Key(named, mods))
}

impl Input<'_> {
    pub fn send(&self, client: &AttachClient) -> Result<(), TransportError> {
        match *self {
            Self::Key(key, mods) => client.send_key(key, mods),
            Self::Text(text) if client.session_protocol_version() >= TEXT_INPUT_MIN_VERSION => {
                client.send_text_input(text)
            }
            Self::Text(text) => {
                for c in text.chars() {
                    client.send_key(Key::Char(c), Modifiers::NONE)?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers as NativeMods;

    fn stroke(key: &str, text: Option<&str>, modifiers: NativeMods) -> Keystroke {
        Keystroke {
            key: key.into(),
            key_char: text.map(str::to_owned),
            modifiers,
        }
    }

    #[test]
    fn chords_use_the_key_and_printable_input_uses_the_resolved_layout() {
        let alt = NativeMods {
            alt: true,
            ..Default::default()
        };
        let ctrl = NativeMods {
            control: true,
            ..Default::default()
        };
        let shift = NativeMods {
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            translate(&stroke("x", Some("≈"), alt)),
            Some(Input::Key(Key::Char('x'), Modifiers::ALT))
        );
        assert_eq!(
            translate(&stroke("/", None, ctrl)),
            Some(Input::Key(Key::Char('/'), Modifiers::CTRL))
        );
        assert_eq!(
            translate(&stroke("a", Some("A"), shift)),
            Some(Input::Key(Key::Char('A'), Modifiers::NONE))
        );
        assert_eq!(
            translate(&stroke("a", Some("界"), Default::default())),
            Some(Input::Key(Key::Char('界'), Modifiers::NONE))
        );
        assert_eq!(
            translate(&stroke("a", Some("e\u{301}"), Default::default())),
            Some(Input::Text("e\u{301}"))
        );
        assert_eq!(
            translate(&stroke("enter", Some("\r"), Default::default())),
            Some(Input::Key(Key::Enter, Modifiers::NONE))
        );
        assert_eq!(
            translate(&stroke("dead_acute", None, Default::default())),
            None
        );
    }
}
