//! The GitHub API through `curl`, so the binary carries no TLS stack. The
//! token goes to curl on stdin, never in its arguments, and redirects are
//! never followed with it: a log's storage URL is fetched without the token.

use std::io::Write as _;
use std::process::{Command, Stdio};

use serde_json::Value;

/// A response: its status and body, GitHub's `message` when it sent one,
/// and the headers that say whether and how long to wait.
#[derive(Debug, Clone, Default)]
pub struct Reply<T> {
    pub status: u16,
    pub body: T,
    pub message: String,
    pub retry_after: Option<u64>,
    pub remaining: Option<u64>,
}

impl<T> Reply<T> {
    pub fn new(status: u16, body: T) -> Reply<T> {
        Reply {
            status,
            body,
            message: String::new(),
            retry_after: None,
            remaining: None,
        }
    }
}

pub trait Http {
    /// The JSON at an API path such as `/repos/o/r/actions/runs`.
    fn get_json(&self, path: &str) -> Result<Reply<Value>, String>;
    /// A job's raw log: `Some` with status 200, `None` with 404 or 410 when
    /// GitHub no longer has it, or the API's refusal.
    fn get_log(&self, repo: &str, job_id: u64) -> Result<Reply<Option<String>>, String>;
    /// Waits before a retry.
    fn pause(&self, seconds: u64) {
        std::thread::sleep(std::time::Duration::from_secs(seconds));
    }
}

pub struct Curl {
    pub token: Option<String>,
    pub api: String,
}

impl Curl {
    pub fn from_env() -> Curl {
        let token = std::env::var("GITHUB_TOKEN")
            .or_else(|_| std::env::var("GH_TOKEN"))
            .ok();
        Curl {
            token,
            api: "https://api.github.com".into(),
        }
    }

    /// Headers, then body, for `url`; the token only for the API host.
    fn request(&self, url: &str, with_token: bool) -> Result<(u16, String, String), String> {
        let mut config = String::from("silent\nshow-error\ninclude\nsuppress-connect-headers\nheader = \"Accept: application/vnd.github+json\"\nheader = \"X-GitHub-Api-Version: 2022-11-28\"\n");
        if let (true, Some(token)) = (with_token, &self.token) {
            config.push_str(&format!("header = \"Authorization: Bearer {token}\"\n"));
        }
        config.push_str(&format!("url = \"{url}\"\n"));
        let mut child = Command::new("curl")
            // `-q` first: no `.curlrc` can add redirects or change the output.
            .args(["-q", "--config", "-", "--max-time", "120"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("couldn't run curl: {e}"))?;
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(config.as_bytes())
            .map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "curl {url}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (status, head, body) = split_response(&text);
        Ok((status, head.to_string(), body.to_string()))
    }
}

fn status_of(head: &str) -> u16 {
    head.lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Status, headers and body of the final response. A proxy's `CONNECT`
/// reply or an interim `1xx` can precede it when curl includes headers.
fn split_response(text: &str) -> (u16, &str, &str) {
    let mut rest = text;
    loop {
        let (head, body) = rest.split_once("\r\n\r\n").unwrap_or((rest, ""));
        let status = status_of(head);
        let interim = (100..200).contains(&status)
            || (status == 200
                && head
                    .lines()
                    .next()
                    .is_some_and(|l| l.to_ascii_lowercase().contains("connection established")));
        if interim && body.starts_with("HTTP/") {
            rest = body;
            continue;
        }
        return (status, head, body);
    }
}

fn header(head: &str, name: &str) -> Option<String> {
    head.lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.trim().eq_ignore_ascii_case(name))
        })
        .map(|(_, v)| v.trim().to_string())
}

fn location(head: &str) -> Option<String> {
    header(head, "location")
}

fn reply<T>(status: u16, head: &str, raw: &str, body: T) -> Reply<T> {
    let message = serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    Reply {
        status,
        body,
        message,
        retry_after: header(head, "retry-after").and_then(|v| v.parse().ok()),
        remaining: header(head, "x-ratelimit-remaining").and_then(|v| v.parse().ok()),
    }
}

impl Http for Curl {
    fn get_json(&self, path: &str) -> Result<Reply<Value>, String> {
        let (status, head, body) = self.request(&format!("{}{path}", self.api), true)?;
        let value = serde_json::from_str(&body).unwrap_or(Value::Null);
        Ok(reply(status, &head, &body, value))
    }

    fn get_log(&self, repo: &str, job_id: u64) -> Result<Reply<Option<String>>, String> {
        let (status, head, body) = self.request(
            &format!("{}/repos/{repo}/actions/jobs/{job_id}/logs", self.api),
            true,
        )?;
        Ok(match status {
            200 => Reply::new(200, Some(body)),
            // Log storage refusing a signed URL means the log is gone.
            301 | 302 | 303 | 307 | 308 => match location(&head) {
                Some(url) => match self.request(&url, false)? {
                    (200, _, body) => Reply::new(200, Some(body)),
                    _ => Reply::new(410, None),
                },
                None => Reply::new(410, None),
            },
            403 | 429 => reply(status, &head, &body, None),
            _ => Reply::new(410, None),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proxy_connect_reply_is_skipped_before_the_response() {
        let text =
            "HTTP/1.1 200 Connection Established\r\n\r\nHTTP/1.1 404 Not Found\r\nx: y\r\n\r\n{}";
        assert_eq!(
            split_response(text),
            (404, "HTTP/1.1 404 Not Found\r\nx: y", "{}")
        );
        let plain = "HTTP/2 200\r\nx: y\r\n\r\nHTTP/1.1 is in the body";
        assert_eq!(split_response(plain).0, 200);
        assert_eq!(split_response(plain).2, "HTTP/1.1 is in the body");
    }

    #[test]
    fn a_refusal_carries_its_message_and_wait_headers() {
        let head = "HTTP/2 403\r\nRetry-After: 42\r\nx-ratelimit-remaining: 17";
        let r = reply(
            403,
            head,
            r#"{"message":"You have exceeded a secondary rate limit."}"#,
            (),
        );
        assert_eq!((r.retry_after, r.remaining), (Some(42), Some(17)));
        assert!(r.message.contains("secondary rate limit"));
    }

    #[test]
    fn the_location_header_is_found_whatever_its_case() {
        let head = "HTTP/2 302\r\nlocation: https://storage.example/log?sig=x\r\nx: y";
        assert_eq!(
            location(head).as_deref(),
            Some("https://storage.example/log?sig=x")
        );
    }
}
