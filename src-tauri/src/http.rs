//! Just enough HTTP/1.1 for Bangs' two small APIs: the loopback one plugins
//! post to (activities.rs) and the one the Apple Watch app talks to on the
//! local network (remote.rs). One request per connection, no keep-alive, no
//! chunked bodies — every client either is curl or was written for this.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub const IO_TIMEOUT: Duration = Duration::from_secs(3);
/// All the time one request gets, so a client trickling a byte at a time
/// cannot hold a connection (and its thread) open for long.
const REQUEST_DEADLINE: Duration = Duration::from_secs(10);

pub struct Request {
    pub method: String,
    /// The path without its query string.
    pub path: String,
    headers: Vec<(String, String)>,
    pub body: String,
}

impl Request {
    /// The first header called `name`, which must be given in lowercase.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Reads one request, refusing anything larger than `max` bytes in all.
pub fn read_request(stream: &mut TcpStream, max: usize) -> Option<Request> {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let deadline = Instant::now() + REQUEST_DEADLINE;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    // Headers first: read until the blank line that ends them.
    let head_end = loop {
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 || buffer.len() + read > max || Instant::now() > deadline {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
    let mut lines = head.lines();
    let mut start = lines.next()?.split_whitespace();
    let method = start.next()?.to_string();
    let target = start.next()?;
    let path = target.split('?').next().unwrap_or_default().to_string();
    let mut headers = Vec::new();
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if name == "content-length" {
            length = value.parse().unwrap_or(0);
        }
        headers.push((name, value));
    }
    let mut body = buffer[head_end..].to_vec();
    while body.len() < length.min(max) {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 || Instant::now() > deadline {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Some(Request {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

pub fn reply(stream: &mut TcpStream, status: &str, body: &str) {
    reply_bytes(stream, status, "application/json; charset=utf-8", body.as_bytes());
}

pub fn reply_bytes(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// `{"error": message}`, with the message escaped.
pub fn error(stream: &mut TcpStream, status: &str, message: &str) {
    let body = serde_json::json!({ "error": message }).to_string();
    reply(stream, status, &body);
}

/// 64 hex characters from the OS's entropy. RandomState seeds itself from the
/// OS, which is the entropy std exposes without pulling in a crate.
pub fn random_token() -> String {
    (0..4).map(|_| format!("{:016x}", random_u64())).collect()
}

pub fn random_u64() -> u64 {
    RandomState::new().build_hasher().finish()
}

/// Compares secrets without stopping at the first difference, so the time a
/// wrong guess takes says nothing about how close it was.
pub fn same_secret(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_and_different() {
        let a = random_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, random_token());
    }

    #[test]
    fn secrets_compare_by_content() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("abc", "abcd"));
    }
}
