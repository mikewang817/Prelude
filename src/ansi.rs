//! Terminal escape codes. Kept as plain constants so they cost nothing.
pub const DIM: &str = "\x1b[2m";
pub const RESET: &str = "\x1b[0m";
pub const CYAN: &str = "\x1b[36m";
pub const RED: &str = "\x1b[31m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const BLUE: &str = "\x1b[34m";
pub const MAGENTA: &str = "\x1b[35m";
/// Reserved for Quicklinks. Every other colour already names a source; a
/// quicklink is the one row kind the person created themselves, and it has to
/// be tellable apart from the file, folder or link it happens to point at.
pub const BRIGHT_CYAN: &str = "\x1b[96m";
