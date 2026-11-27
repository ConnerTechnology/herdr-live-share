//! Small helpers: hex, base32, hashing, time.

use sha2::{Digest, Sha256};

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let d = Sha256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_slice());
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// Lower-case RFC 4648 base32 without padding.
pub fn base32(bytes: &[u8]) -> String {
    const A: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let (mut out, mut buf, mut bits) = (String::new(), 0u32, 0u32);
    for &b in bytes {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(A[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(A[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// "4 min", "2 h", "3 days".
pub fn ago(ms: u64) -> String {
    let min = ms / 60_000;
    match min {
        0 => "just now".into(),
        1..=59 => format!("{min} min ago"),
        60..=2879 => format!("{} h ago", min / 60),
        _ => format!("{} days ago", min / 1440),
    }
}

/// 32 random bytes from the OS (Linux and macOS).
pub fn random32() -> std::io::Result<[u8; 32]> {
    use std::io::Read;
    let mut b = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b)
}

/// Run this binary with `args`, detached: its own process group, so it
/// outlives the hook that started it; stdout and stderr appended to `log`.
pub fn spawn_detached(args: &[String], log: &std::path::Path) {
    let Ok(exe) = std::env::current_exe() else { return };
    let _ = std::fs::create_dir_all(log.parent().unwrap_or(std::path::Path::new(".")));
    let out = std::fs::File::options().create(true).append(true).open(log).ok();
    let mut c = std::process::Command::new(exe);
    c.args(args).stdin(std::process::Stdio::null());
    match out.and_then(|o| o.try_clone().ok().map(|e| (o, e))) {
        Some((o, e)) => {
            c.stdout(o).stderr(e);
        }
        None => {
            c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let _ = c.spawn();
}
