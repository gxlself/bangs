//! Just enough HTTP/1.1 for the two small APIs Bangs serves: the loopback
//! board API (activities.rs) and the watch API on the local network (watch.rs).
//! One request per connection, no keep-alive, no chunked bodies.

use std::io::{Read, Write};
use std::net::TcpStream;

/// Largest request, headers and body together, that either API accepts.
pub const MAX_BODY: usize = 32 * 1024;

pub struct Request {
    pub method: String,
    /// The path as sent, query string included.
    pub path: String,
    /// `X-Bangs-Token`, the board API's secret.
    pub token: Option<String>,
    /// `Authorization`, as the watch sends it (`Bearer …`).
    pub authorization: Option<String>,
    /// Browsers set this; command line clients and the watch do not.
    pub origin: Option<String>,
    pub body: String,
}

impl Request {
    /// The path without its query string.
    pub fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or_default()
    }

    /// The value of `name` in the query string, if there is one.
    pub fn query(&self, name: &str) -> Option<&str> {
        let (_, query) = self.path.split_once('?')?;
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    }
}

pub fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    // Headers first: read until the blank line that ends them.
    let head_end = loop {
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 || buffer.len() + read > MAX_BODY {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
    let mut lines = head.lines();
    let mut start = lines.next()?.split_whitespace();
    let method = start.next()?.to_string();
    let path = start.next()?.to_string();
    let mut token = None;
    let mut authorization = None;
    let mut origin = None;
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "x-bangs-token" => token = Some(value.to_string()),
            "authorization" => authorization = Some(value.to_string()),
            "origin" => origin = Some(value.to_string()),
            "content-length" => length = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    let mut body = buffer[head_end..].to_vec();
    while body.len() < length.min(MAX_BODY) {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Some(Request {
        method,
        path,
        token,
        authorization,
        origin,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

pub fn reply(stream: &mut TcpStream, status: &str, body: &str) {
    reply_bytes(stream, status, "application/json; charset=utf-8", body.as_bytes());
}

pub fn reply_bytes(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}
