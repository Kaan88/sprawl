//! A small blocking HTTP POST over WinHTTP: Windows' own TLS and certificate store, and the
//! system proxy settings. Declared by hand (no `windows-sys` features: the build's command line
//! is already at its limit).

use std::time::Duration;

/// An HTTP answer: status code, `Retry-After` in seconds if given, and the body.
#[derive(Debug)]
pub struct Response {
    pub status: u32,
    pub retry_after: Option<u64>,
    pub body: String,
}

/// Split `http[s]://host[:port]/path` into (secure, host, port, path).
pub(crate) fn split_url(url: &str) -> Result<(bool, String, u16, String), String> {
    let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(format!("not an http(s) address: {url}"));
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.contains(']') || h.ends_with(']') => (h, p.parse::<u16>().map_err(|_| format!("bad port in {url}"))?),
        _ => (authority, if secure { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(format!("no host in {url}"));
    }
    Ok((secure, host.trim_matches(['[', ']']).to_string(), port, path.to_string()))
}

#[cfg(windows)]
mod ffi {
    use std::ffi::c_void;

    pub type H = *mut c_void;

    #[link(name = "winhttp")]
    extern "system" {
        pub fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> H;
        pub fn WinHttpSetTimeouts(h: H, resolve: i32, connect: i32, send: i32, receive: i32) -> i32;
        pub fn WinHttpConnect(h: H, server: *const u16, port: u16, reserved: u32) -> H;
        pub fn WinHttpOpenRequest(h: H, verb: *const u16, object: *const u16, version: *const u16, referrer: *const u16, accept: *const *const u16, flags: u32) -> H;
        pub fn WinHttpAddRequestHeaders(h: H, headers: *const u16, len: u32, modifiers: u32) -> i32;
        pub fn WinHttpSendRequest(h: H, headers: *const u16, hlen: u32, optional: *const c_void, olen: u32, total: u32, ctx: usize) -> i32;
        pub fn WinHttpReceiveResponse(h: H, reserved: *mut c_void) -> i32;
        pub fn WinHttpQueryHeaders(h: H, info: u32, name: *const u16, buf: *mut c_void, len: *mut u32, index: *mut u32) -> i32;
        pub fn WinHttpQueryDataAvailable(h: H, avail: *mut u32) -> i32;
        pub fn WinHttpReadData(h: H, buf: *mut c_void, len: u32, read: *mut u32) -> i32;
        pub fn WinHttpCloseHandle(h: H) -> i32;
        pub fn WinHttpSetOption(h: H, option: u32, buf: *const c_void, len: u32) -> i32;
    }

    pub const OPTION_REDIRECT_POLICY: u32 = 88;
    pub const REDIRECT_POLICY_NEVER: u32 = 0;

    pub const ACCESS_AUTOMATIC_PROXY: u32 = 4;
    pub const ACCESS_DEFAULT_PROXY: u32 = 0;
    pub const FLAG_SECURE: u32 = 0x0080_0000;
    pub const ADDREQ_ADD_REPLACE: u32 = 0x2000_0000 | 0x8000_0000;
    pub const QUERY_STATUS_CODE: u32 = 19;
    pub const QUERY_CUSTOM: u32 = 65535;
    pub const QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

    /// Closes the handle when dropped.
    pub struct Handle(pub H);

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn last_error(what: &str) -> String {
    let e = std::io::Error::last_os_error();
    let code = e.raw_os_error().unwrap_or(0);
    let why = match code {
        12002 => "timed out".to_string(),
        12007 => "the server name couldn't be resolved".to_string(),
        12029 => "couldn't connect".to_string(),
        12030 => "the connection was dropped".to_string(),
        12175 => "a TLS or certificate error".to_string(),
        _ => format!("error {code}"),
    };
    format!("{what}: {why}")
}

/// POST `body` (JSON) to `url` with `headers`.
pub fn post(url: &str, headers: &[(&str, String)], body: &[u8], timeout: Duration) -> Result<Response, String> {
    request("POST", url, headers, Some(body), timeout, None)
}

/// POST, handing each piece of a successful answer to `chunk` as it arrives (an answer that
/// isn't 2xx is collected into `Response::body` as usual).
pub fn post_stream(url: &str, headers: &[(&str, String)], body: &[u8], timeout: Duration, chunk: &mut dyn FnMut(&[u8])) -> Result<Response, String> {
    request("POST", url, headers, Some(body), timeout, Some(chunk))
}

/// GET `url`.
pub fn get(url: &str, headers: &[(&str, String)], timeout: Duration) -> Result<Response, String> {
    request("GET", url, headers, None, timeout, None)
}

/// Splits a server-sent event stream into its `data:` payloads.
#[derive(Default)]
pub(crate) struct Sse {
    buf: Vec<u8>,
}

impl Sse {
    /// Feed bytes; returns the complete `data:` lines so far.
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(d) = line.strip_prefix("data:") {
                out.push(d.trim_start().to_string());
            }
        }
        out
    }
}

#[cfg(windows)]
fn request(verb: &str, url: &str, headers: &[(&str, String)], body: Option<&[u8]>, timeout: Duration, mut chunk: Option<&mut dyn FnMut(&[u8])>) -> Result<Response, String> {
    use ffi::*;
    use std::ptr::null;

    let (secure, host, port, path) = split_url(url)?;
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    unsafe {
        let agent = wide("Sprawl");
        let mut session = Handle(WinHttpOpen(agent.as_ptr(), ACCESS_AUTOMATIC_PROXY, null(), null(), 0));
        if session.0.is_null() {
            // before Windows 8.1 there is no automatic proxy mode
            session = Handle(WinHttpOpen(agent.as_ptr(), ACCESS_DEFAULT_PROXY, null(), null(), 0));
        }
        if session.0.is_null() {
            return Err(last_error("WinHTTP"));
        }
        WinHttpSetTimeouts(session.0, 20_000, 20_000, ms, ms);
        // never follow a redirect: the key would go wherever it points
        let never = REDIRECT_POLICY_NEVER;
        if WinHttpSetOption(session.0, OPTION_REDIRECT_POLICY, (&never as *const u32).cast(), 4) == 0 {
            return Err(last_error("turning off redirects"));
        }
        let host_w = wide(&host);
        let connect = Handle(WinHttpConnect(session.0, host_w.as_ptr(), port, 0));
        if connect.0.is_null() {
            return Err(last_error(&format!("connecting to {host}")));
        }
        let verb = wide(verb);
        let path_w = wide(&path);
        let request = Handle(WinHttpOpenRequest(connect.0, verb.as_ptr(), path_w.as_ptr(), null(), null(), null(), if secure { FLAG_SECURE } else { 0 }));
        if request.0.is_null() {
            return Err(last_error("opening the request"));
        }
        let mut all = String::from(if body.is_some() { "Content-Type: application/json\r\n" } else { "" });
        for (k, v) in headers {
            // a header value can't break into another header
            all.push_str(&format!("{k}: {}\r\n", v.replace(['\r', '\n'], "")));
        }
        if !all.is_empty() {
            let hw = wide(&all);
            if WinHttpAddRequestHeaders(request.0, hw.as_ptr(), u32::MAX, ADDREQ_ADD_REPLACE) == 0 {
                return Err(last_error("adding headers"));
            }
        }
        let body = body.unwrap_or(&[]);
        let len = u32::try_from(body.len()).map_err(|_| "request too big".to_string())?;
        if WinHttpSendRequest(request.0, null(), 0, body.as_ptr().cast(), len, len, 0) == 0 {
            return Err(last_error(&format!("sending to {host}")));
        }
        if WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0 {
            return Err(last_error(&format!("waiting for {host}")));
        }
        let number = |what: u32| -> Option<u32> {
            let mut v = 0u32;
            let mut size = 4u32;
            (WinHttpQueryHeaders(request.0, what | QUERY_FLAG_NUMBER, null(), (&mut v as *mut u32).cast(), &mut size, std::ptr::null_mut()) != 0).then_some(v)
        };
        let status = number(QUERY_STATUS_CODE).ok_or_else(|| last_error("reading the status"))?;
        // Retry-After as text, by name: seconds (an HTTP date is ignored)
        let retry_after = {
            let name = wide("Retry-After");
            let mut buf = [0u16; 64];
            let mut size = (buf.len() * 2) as u32;
            (WinHttpQueryHeaders(request.0, QUERY_CUSTOM, name.as_ptr(), buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut()) != 0)
                .then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).min(buf.len())]))
                .and_then(|s| s.trim().parse::<u64>().ok())
        };
        // only a good answer streams; an error body is read whole, to be shown
        if !(200..300).contains(&status) {
            chunk = None;
        }
        let mut out = Vec::new();
        let mut streamed = 0usize;
        loop {
            let mut avail = 0u32;
            if WinHttpQueryDataAvailable(request.0, &mut avail) == 0 {
                return Err(last_error("reading the answer"));
            }
            if avail == 0 {
                break;
            }
            // answers are small; refuse anything absurd
            if out.len() + streamed + avail as usize > 64 << 20 {
                return Err("the answer is over 64 MB".into());
            }
            let start = out.len();
            out.resize(start + avail as usize, 0);
            let mut read = 0u32;
            if WinHttpReadData(request.0, out[start..].as_mut_ptr().cast(), avail, &mut read) == 0 {
                return Err(last_error("reading the answer"));
            }
            out.truncate(start + read as usize);
            if let Some(c) = chunk.as_mut() {
                c(&out[start..]);
                streamed += out.len() - start;
                out.truncate(start);
            }
            if read == 0 {
                break;
            }
        }
        Ok(Response { status, retry_after, body: String::from_utf8_lossy(&out).into_owned() })
    }
}

#[cfg(not(windows))]
fn request(_: &str, _: &str, _: &[(&str, String)], _: Option<&[u8]>, _: Duration, _: Option<&mut dyn FnMut(&[u8])>) -> Result<Response, String> {
    Err("HTTP is only built for Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    #[test]
    fn sse_lines() {
        let mut s = Sse::default();
        assert!(s.feed(b"event: x\ndata: {\"a\":").is_empty());
        assert_eq!(s.feed(b"1}\r\n\ndata: [DONE]\n"), vec!["{\"a\":1}".to_string(), "[DONE]".to_string()]);
    }

    #[test]
    fn urls() {
        assert_eq!(split_url("https://api.anthropic.com/v1/messages").unwrap(), (true, "api.anthropic.com".into(), 443, "/v1/messages".into()));
        assert_eq!(split_url("http://localhost:11434/v1/chat/completions").unwrap(), (false, "localhost".into(), 11434, "/v1/chat/completions".into()));
        assert_eq!(split_url("http://127.0.0.1:1234").unwrap().3, "/");
        assert!(split_url("ftp://x").is_err() && split_url("http://:80/").is_err());
    }

    #[test]
    fn posts_to_a_local_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut len = 0;
            let mut saw_key = false;
            loop {
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let l = line.to_ascii_lowercase();
                if let Some(v) = l.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                saw_key |= l.starts_with("x-api-key: secret");
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            let reply = format!("{{\"echo\": {}, \"key\": {saw_key}}}", String::from_utf8(body).unwrap());
            write!(s, "HTTP/1.1 429 Too Many\r\nRetry-After: 7\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
        });
        let r = post(&format!("http://127.0.0.1:{port}/v1/x"), &[("x-api-key", "secret\r\nEvil: 1".into())], b"{\"a\":1}", Duration::from_secs(10)).unwrap();
        server.join().unwrap();
        assert_eq!(r.status, 429);
        assert_eq!(r.retry_after, Some(7));
        assert!(r.body.contains("\"echo\": {\"a\":1}") && r.body.contains("\"key\": true"), "{}", r.body);
    }
}
