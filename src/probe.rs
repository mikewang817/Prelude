//! Ask the terminal how wide an ambiguous character actually is.
//!
//! Beats inferring from $LANG — this measures what actually happened.

use std::io::{Read, Write};

pub fn ambiguous_width() -> Option<usize> {
    let mut tty_out = std::fs::OpenOptions::new().write(true).open("/dev/tty").ok()?;
    let mut tty_in = std::fs::File::open("/dev/tty").ok()?;
    let saved = crate::tty::raw_mode()?;

    let result = (|| -> Option<usize> {
        tty_out.write_all(b"\r").ok()?;
        tty_out.write_all("·".as_bytes()).ok()?; // the character under test
        tty_out.write_all(b"\x1b[6n").ok()?; // cursor position report
        tty_out.flush().ok()?;

        let mut resp = Vec::new();
        let mut buf = [0u8; 32];
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline && !resp.contains(&b'R') {
            match tty_in.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => resp.extend_from_slice(&buf[..n]),
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        let _ = tty_out.write_all(b"\r \r"); // wipe the probe character
        let s = String::from_utf8_lossy(&resp);
        let (_, after) = s.split_once(';')?;
        let col: usize = after.trim_end_matches(['R', '\u{1b}', '[']).trim().parse().ok()?;
        Some((col.saturating_sub(1)).clamp(1, 2))
    })();

    crate::tty::restore(Some(saved));
    result
}
