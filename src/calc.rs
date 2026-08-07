//! Arithmetic and date arithmetic.
//!
//! Rust has no `eval`, so this is a small recursive-descent parser. That is
//! not a downside: the Python version had to be careful to compile the
//! expression with no names and no builtins available, whereas here nothing
//! but numbers and operators can be expressed in the first place.

/// Evaluate an arithmetic query. `None` if it isn't one.
pub fn calc(query: &str) -> Option<String> {
    let q = query.trim();
    if q.len() < 3 || !q.contains(['+', '-', '*', '/', '%']) {
        return None;
    }
    if !q.chars().all(|c| c.is_ascii_digit() || " .+-*/%()e".contains(c)) {
        return None;
    }
    let mut p = Parser { b: q.as_bytes(), i: 0 };
    let v = p.expr()?;
    p.ws();
    if p.i != p.b.len() {
        return None;
    }
    Some(fmt_num(v))
}

pub fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    if (v - v.round()).abs() < 1e-9 && v.abs() < 1e15 {
        group(&format!("{}", v.round() as i64))
    } else {
        let s = format!("{v:.6}");
        let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        match s.split_once('.') {
            Some((a, b)) => format!("{}.{}", group(a), b),
            None => group(&s),
        }
    }
}

/// Thousands separators, matching the Python formatting.
fn group(n: &str) -> String {
    let (sign, digits) = match n.strip_prefix('-') {
        Some(d) => ("-", d),
        None => ("", n),
    };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i] == b' ' {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.b.get(self.i).copied()
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(op) = self.peek() {
            if op != b'+' && op != b'-' {
                break;
            }
            self.i += 1;
            let r = self.term()?;
            v = if op == b'+' { v + r } else { v - r };
        }
        Some(v)
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        while let Some(op) = self.peek() {
            if !matches!(op, b'*' | b'/' | b'%') {
                break;
            }
            self.i += 1;
            let r = self.unary()?;
            v = match op {
                b'*' => v * r,
                b'/' => {
                    if r == 0.0 {
                        return None;
                    }
                    v / r
                }
                _ => {
                    if r == 0.0 {
                        return None;
                    }
                    v % r
                }
            };
        }
        Some(v)
    }
    fn unary(&mut self) -> Option<f64> {
        match self.peek()? {
            b'-' => {
                self.i += 1;
                Some(-self.unary()?)
            }
            b'+' => {
                self.i += 1;
                self.unary()
            }
            _ => self.atom(),
        }
    }
    fn atom(&mut self) -> Option<f64> {
        if self.peek()? == b'(' {
            self.i += 1;
            let v = self.expr()?;
            if self.peek()? != b')' {
                return None;
            }
            self.i += 1;
            return Some(v);
        }
        self.ws();
        let start = self.i;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            let exp_sign = (c == b'+' || c == b'-')
                && self.i > start
                && (self.b[self.i - 1] | 32) == b'e';
            if c.is_ascii_digit() || c == b'.' || (c | 32) == b'e' || exp_sign {
                self.i += 1;
            } else {
                break;
            }
        }
        std::str::from_utf8(&self.b[start..self.i]).ok()?.parse().ok()
    }
}

// ─── dates ───────────────────────────────────────────────────────────────
// std has no calendar, and pulling in chrono for one format string would
// cost more startup than the whole feature is worth.

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Seconds the local zone is ahead of UTC, discovered by asking libc once.
fn utc_offset() -> i64 {
    use std::sync::OnceLock;
    static OFF: OnceLock<i64> = OnceLock::new();
    *OFF.get_or_init(|| {
        let out = crate::exec::run(&["date", "+%z"], std::time::Duration::from_secs(2));
        let s = out.trim();
        if s.len() < 5 {
            return 0;
        }
        let sign = if s.starts_with('-') { -1 } else { 1 };
        let h: i64 = s[1..3].parse().unwrap_or(0);
        let m: i64 = s[3..5].parse().unwrap_or(0);
        sign * (h * 3600 + m * 60)
    })
}

/// Howard Hinnant's civil-from-days algorithm.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn fmt_local(epoch: i64) -> String {
    let t = epoch + utc_offset();
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let (y, mo, d) = civil(days);
    format!(
        "{y:04}-{mo:02}-{d:02} {:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

fn midnight(epoch: i64) -> i64 {
    let t = epoch + utc_offset();
    t - t.rem_euclid(86_400) - utc_offset()
}

/// `now + 3 days`, or a bare unix timestamp.
pub fn timecalc(query: &str) -> Option<(String, String)> {
    let q = query.trim();

    if (q.len() == 10 || q.len() == 13) && q.bytes().all(|c| c.is_ascii_digit()) {
        let raw: i64 = q.parse().ok()?;
        let secs = if q.len() == 13 { raw / 1000 } else { raw };
        return Some((fmt_local(secs), format!("timestamp {raw} · local time")));
    }

    let low = q.to_ascii_lowercase();
    let mut parts = low.split_whitespace();
    let base_word = parts.next()?;
    if base_word != "now" && base_word != "today" {
        return None;
    }
    let op = parts.next()?;
    let sign: i64 = match op {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let n: i64 = parts.next()?.parse().ok()?;
    let unit = parts.next()?.trim_end_matches('s');
    if parts.next().is_some() {
        return None;
    }
    let mult: i64 = match unit {
        "second" | "sec" => 1,
        "minute" | "min" => 60,
        "hour" | "hr" => 3600,
        "day" => 86_400,
        "week" => 604_800,
        "month" => 2_592_000,
        "year" => 31_536_000,
        _ => return None,
    };
    let base = if base_word == "today" { midnight(now_secs()) } else { now_secs() };
    let plural = if n == 1 { "" } else { "s" };
    Some((
        fmt_local(base + sign * n * mult),
        format!("{base_word} {op} {n} {unit}{plural}"),
    ))
}
