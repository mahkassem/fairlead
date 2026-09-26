//! The GitHub API through `curl`, so the binary carries no TLS stack. The
//! token goes to curl on stdin, never in its arguments, and redirects are
//! never followed with it: a log's storage URL is fetched without the token.

use std::io::Write as _;
use std::process::{Command, Stdio};

use serde_json::Value;

pub trait Http {
    /// `(status, body)` for an API path such as `/repos/o/r/actions/runs`.
    fn get_json(&self, path: &str) -> Result<(u16, Value), String>;
    /// A job's raw log, or `None` when GitHub no longer has it.
    fn get_log(&self, repo: &str, job_id: u64) -> Result<Option<String>, String>;
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
            .args(["--config", "-", "--max-time", "120"])
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

fn location(head: &str) -> Option<String> {
    head.lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("location"))
        })
        .map(|(_, v)| v.trim().to_string())
}

impl Http for Curl {
    fn get_json(&self, path: &str) -> Result<(u16, Value), String> {
        let (status, _, body) = self.request(&format!("{}{path}", self.api), true)?;
        let value = serde_json::from_str(&body).unwrap_or(Value::Null);
        Ok((status, value))
    }

    fn get_log(&self, repo: &str, job_id: u64) -> Result<Option<String>, String> {
        let (status, head, body) = self.request(
            &format!("{}/repos/{repo}/actions/jobs/{job_id}/logs", self.api),
            true,
        )?;
        match status {
            200 => Ok(Some(body)),
            301 | 302 | 303 | 307 | 308 => match location(&head) {
                Some(url) => {
                    let (status, _, body) = self.request(&url, false)?;
                    Ok((status == 200).then_some(body))
                }
                None => Ok(None),
            },
            _ => Ok(None),
        }
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
    fn the_location_header_is_found_whatever_its_case() {
        let head = "HTTP/2 302\r\nlocation: https://storage.example/log?sig=x\r\nx: y";
        assert_eq!(
            location(head).as_deref(),
            Some("https://storage.example/log?sig=x")
        );
    }
}
