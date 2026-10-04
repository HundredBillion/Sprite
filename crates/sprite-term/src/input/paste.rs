use crate::SessionError;
use libghostty_vt::Terminal;
const MODE_BRACKETED_PASTE: u16 = 2004;
/// Whether a paste can be performed without asking.
///
/// Bracketed paste is safe by construction: the child is told where the text
/// begins and ends, so a newline inside it is data. Without bracketing, a
/// newline arrives as if typed — Sprite writes a carriage return, but the line
/// discipline converts it straight back — so anything libghostty considers
/// unsafe needs a person's confirmation first.
pub(crate) fn paste_is_safe_to_perform(terminal: &Terminal<'_, '_>, text: &str) -> bool {
    use libghostty_vt::paste;
    use libghostty_vt::terminal::{Mode, ModeKind};

    let bracketed = terminal
        .mode(Mode::new(MODE_BRACKETED_PASTE, ModeKind::Dec))
        .unwrap_or(false);
    bracketed || paste::is_safe(text)
}

/// Prepares clipboard text for the PTY.
///
/// libghostty does the dangerous part: it strips control bytes, neutralises a
/// payload's attempt to close bracketed paste early, and converts newlines to
/// carriage returns when the child is not bracketing. Sprite must not
/// reimplement any of that.
pub(crate) fn encode_paste(
    terminal: &Terminal<'_, '_>,
    text: &str,
) -> Result<Vec<u8>, SessionError> {
    use libghostty_vt::paste;
    use libghostty_vt::terminal::{Mode, ModeKind};

    let bracketed = terminal
        .mode(Mode::new(MODE_BRACKETED_PASTE, ModeKind::Dec))
        .map_err(|error| SessionError::new("paste_mode", error))?;

    let mut data = text.as_bytes().to_vec();
    // Bracketing adds a prefix and suffix, and stripping never grows the text,
    // so a generous margin is enough for one attempt.
    let mut buffer = vec![0_u8; data.len() + 64];
    match paste::encode(&mut data, bracketed, &mut buffer) {
        Ok(written) => {
            buffer.truncate(written);
            Ok(buffer)
        }
        Err(libghostty_vt::Error::OutOfSpace { required }) => {
            let mut data = text.as_bytes().to_vec();
            let mut buffer = vec![0_u8; required];
            let written = paste::encode(&mut data, bracketed, &mut buffer)
                .map_err(|error| SessionError::new("paste_encode", error))?;
            buffer.truncate(written);
            Ok(buffer)
        }
        Err(error) => Err(SessionError::new("paste_encode", error)),
    }
}
