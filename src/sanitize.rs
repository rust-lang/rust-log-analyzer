use regex::bytes::Regex;

pub fn split_lines(data: &[u8]) -> Vec<&[u8]> {
    lazy_static! {
        static ref LINE_BREAK: Regex = Regex::new("[\\r\\n]").unwrap();
    }

    LINE_BREAK.split(data).filter(|line| !line.iter().all(|b| b.is_ascii_whitespace())).collect()
}

/// Cleans up the given `data`:
///
/// * Removes most ANSI escape codes from the input.
/// * Replaces all (Unicode) whitespace with single spaces.
/// * Removes all (Unicode) control characters.
#[cfg_attr(feature = "cargo-clippy", allow(invalid_regex))] // Waiting on upstream fix
pub fn clean(data: &[u8]) -> Vec<u8> {
    lazy_static! {
        /// This catches most escape sequences. And I care about neither
        ///
        /// * the very-special cases ("Set Keyboard Strings") that aren't matched properly nor
        /// * legitimate output (which hopefully doesn't exist) that contains `ESC`.
        ///
        /// Reference: http://ascii-table.com/ansi-escape-sequences.php
        static ref ANSI_ESCAPES: Regex = Regex::new("\x1b.*?[a-zA-Z]").unwrap();

        static ref UNICODE_WHITESPACE: Regex = Regex::new("(?u:\\p{White_Space})").unwrap();

        static ref UNICODE_CONTROL: Regex = Regex::new("(?u:\\p{Control})").unwrap();
    }

    let data = ANSI_ESCAPES.replace_all(data, b"".as_ref());
    let data = UNICODE_WHITESPACE.replace_all(&data, b" ".as_ref());
    UNICODE_CONTROL.replace_all(&data, b"".as_ref()).into_owned()
}
