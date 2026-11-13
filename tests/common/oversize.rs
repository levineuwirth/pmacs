//! E9.5's oversize fixture: pseudo-random printable text, which no
//! snapshot encoding packs, so a buffer of twenty-four MiB of it has a
//! snapshot past `pmacs_protocol::MAX_FRAME_BYTES` (sixteen MiB; about
//! twice the text). Extracted at E10t's fix round 2 (D37) from the three
//! copies the E9 suites carried, for the E10t suite, its fourth user; the
//! body is E9's.

/// The seed E9.5's row and E9 review 1's probes write their file from.
pub const OVERSIZE_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// At least `bytes` of the fixture from [`OVERSIZE_SEED`].
pub fn oversize_text(bytes: usize) -> String {
    oversize_text_from(bytes, OVERSIZE_SEED)
}

/// At least `bytes` of printable text from `seed`, a newline about every
/// eighty bytes: xorshift64, every byte one of the 95 printable
/// characters.
pub fn oversize_text_from(bytes: usize, seed: u64) -> String {
    let mut text = String::with_capacity(bytes + 80);
    let mut state = seed;
    while text.len() < bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        for byte in state.to_le_bytes() {
            text.push(char::from(b' ' + byte % 95));
        }
        if text.len() % 80 < 8 {
            text.push('\n');
        }
    }
    text
}
