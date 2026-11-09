//! Checking `[[needs]]` on this machine (#259): a TCP connect, a command's
//! exit status, or a variable, for `fairlead doctor` and the done gate.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fairlead_core::config::{env_names, Config, Need};

/// A missing need is tried once more after this, for a service still starting.
const RETRY_AFTER: Duration = Duration::from_millis(200);

/// What checking one need found.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub id: String,
    pub met: bool,
    /// What the check saw, such as `localhost:1025 refused`.
    pub saw: String,
    pub tries: u32,
    pub millis: u128,
}

/// Checks every need at once, each tried again once if missing.
pub fn check_all(needs: &[Need], root: &Path) -> Vec<Checked> {
    let lookup = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    std::thread::scope(|scope| {
        let handles: Vec<_> = needs
            .iter()
            .map(|need| scope.spawn(move || check_with_retry(need, root, &lookup)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a check doesn't panic"))
            .collect()
    })
}

fn check_with_retry(need: &Need, root: &Path, env: &dyn Fn(&str) -> Option<String>) -> Checked {
    let started = Instant::now();
    let (mut met, mut saw) = check(need, root, env);
    let mut tries = 1;
    if !met {
        std::thread::sleep(RETRY_AFTER);
        (met, saw) = check(need, root, env);
        tries = 2;
    }
    Checked {
        id: need.id.clone(),
        met,
        saw,
        tries,
        millis: started.elapsed().as_millis(),
    }
}

/// One try: whether the need is met, and what was seen.
pub fn check(need: &Need, root: &Path, env: &dyn Fn(&str) -> Option<String>) -> (bool, String) {
    let timeout = Duration::from_millis(need.timeout_ms());
    if let Some(name) = &need.env {
        return match env(name) {
            Some(_) => (true, format!("{name} is set")),
            None => (false, format!("{name} is unset or empty")),
        };
    }
    if !need.command.is_empty() {
        return command(&need.command, root, timeout);
    }
    let seen: Vec<(bool, String)> = need
        .tcp
        .iter()
        .map(|address| match expand(address, env) {
            Ok(address) => tcp(&address, timeout),
            Err(name) => (false, format!("{address} skipped, {name} unset")),
        })
        .collect();
    let met = seen.iter().any(|(up, _)| *up);
    let saw: Vec<String> = seen.into_iter().map(|(_, s)| s).collect();
    (met, saw.join(", "))
}

/// The address with each `{env:NAME}` filled in, or the first unset name.
pub fn expand(address: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = address.to_string();
    for name in env_names(address).unwrap_or_default() {
        let value = env(name).ok_or_else(|| name.to_string())?;
        out = out.replace(&format!("{{env:{name}}}"), &value);
    }
    Ok(out)
}

/// Resolves on its own thread, so a name server that doesn't answer costs at
/// most `timeout`, then connects to each address in what time is left.
fn tcp(address: &str, timeout: Duration) -> (bool, String) {
    let started = Instant::now();
    let (send, receive) = mpsc::channel();
    let owned = address.to_string();
    std::thread::spawn(move || {
        let _ = send.send(
            owned
                .to_socket_addrs()
                .map(|a| a.collect::<Vec<SocketAddr>>()),
        );
    });
    let addrs = match receive.recv_timeout(timeout) {
        Ok(Ok(addrs)) if !addrs.is_empty() => addrs,
        Ok(_) => return (false, format!("{address} no such host")),
        Err(_) => return (false, format!("{address} lookup timed out")),
    };
    let mut last = "timed out";
    for addr in addrs {
        let left = timeout.saturating_sub(started.elapsed());
        if left.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&addr, left) {
            Ok(_) => return (true, format!("{address} up")),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => last = "refused",
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => last = "timed out",
            Err(_) => last = "unreachable",
        }
    }
    (false, format!("{address} {last}"))
}

fn command(argv: &[String], root: &Path, timeout: Duration) -> (bool, String) {
    let shown = format!("`{}`", argv.join(" "));
    let mut cmd = fairlead_core::process::command(&argv[0], root);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (false, format!("{shown} not found"))
        }
        Err(e) => return (false, format!("{shown} didn't start: {e}")),
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return (true, format!("{shown} exit 0")),
            Ok(Some(status)) => {
                let code = status.code().map_or("a signal".into(), |c| c.to_string());
                return (false, format!("{shown} exit {code}"));
            }
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return (
                    false,
                    format!("{shown} timed out after {} ms", timeout.as_millis()),
                );
            }
        }
    }
}

/// Files that container runtimes leave, shown as a hint for a person reading
/// the report; never used to decide anything.
fn container_markers() -> Vec<&'static str> {
    let files = ["/.dockerenv", "/run/.containerenv"];
    let mut found: Vec<&str> = files
        .into_iter()
        .filter(|f| Path::new(f).exists())
        .collect();
    if std::env::var_os("KUBERNETES_SERVICE_HOST").is_some() {
        found.push("KUBERNETES_SERVICE_HOST");
    }
    found
}

/// The `needs` lines of `fairlead doctor`, empty when the config has none.
pub fn doctor_lines(root: &Path, config: &Config) -> String {
    let needs = config.needs.items();
    if needs.is_empty() {
        return String::new();
    }
    let started = Instant::now();
    let checked = check_all(needs, root);
    let mut out = format!(
        "needs: {} checked in {} ms\n",
        checked.len(),
        started.elapsed().as_millis()
    );
    for (c, need) in checked.iter().zip(needs) {
        let state = if c.met { "met" } else { "missing" };
        let tries = if c.tries > 1 { ", 2 tries" } else { "" };
        out.push_str(&format!(
            "  {}: {state}: {} ({} ms{tries})\n",
            c.id, c.saw, c.millis
        ));
        if !c.met {
            if let Some(hint) = &need.hint {
                out.push_str(&format!("    to meet it: {hint}\n"));
            }
            if let Some(proved) = &need.proved_in {
                out.push_str(&format!("    proved in: {proved}\n"));
            }
        }
    }
    let markers = container_markers();
    if !markers.is_empty() {
        out.push_str(&format!(
            "container: looks like one ({}); a hint only, the checks above decide\n",
            markers.join(", ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn need(toml: &str) -> Need {
        toml::from_str(toml).expect("parses")
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn a_listening_port_is_met_and_a_closed_one_is_refused() {
        // Windows tries a refused connection again for about a second, so
        // within the timeout a closed port reads as timed out there.
        let closed_reads = if cfg!(windows) {
            "timed out"
        } else {
            "refused"
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let open = listener.local_addr().unwrap().port();
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let n = need(&format!(
            "id = \"s\"\ntcp = [\"127.0.0.1:{closed}\", \"127.0.0.1:{open}\"]"
        ));
        let (met, saw) = check(&n, Path::new("."), &none);
        assert!(met, "{saw}");
        assert!(saw.contains(closed_reads) && saw.contains("up"), "{saw}");
        let n = need(&format!("id = \"s\"\ntcp = [\"127.0.0.1:{closed}\"]"));
        assert_eq!(
            check(&n, Path::new("."), &none),
            (false, format!("127.0.0.1:{closed} {closed_reads}"))
        );
    }

    #[test]
    fn an_address_naming_an_unset_variable_is_skipped() {
        let n = need("id = \"s\"\ntcp = [\"{env:HOST}:25\"]");
        let (met, saw) = check(&n, Path::new("."), &none);
        assert!(!met);
        assert_eq!(saw, "{env:HOST}:25 skipped, HOST unset");
        let set = |name: &str| (name == "HOST").then(|| "127.0.0.1".to_string());
        assert_eq!(expand("{env:HOST}:25", &set), Ok("127.0.0.1:25".into()));
    }

    #[test]
    fn a_variable_is_met_when_set_and_not_empty() {
        let n = need("id = \"b\"\nenv = \"BROWSER\"");
        let set = |_: &str| Some("/usr/bin/chromium".to_string());
        assert!(check(&n, Path::new("."), &set).0);
        assert!(!check(&n, Path::new("."), &none).0);
    }

    #[cfg(unix)]
    #[test]
    fn a_command_is_met_on_exit_0_and_missing_otherwise() {
        let dir = Path::new(".");
        assert!(check(&need("id = \"c\"\ncommand = [\"true\"]"), dir, &none).0);
        let (met, saw) = check(&need("id = \"c\"\ncommand = [\"false\"]"), dir, &none);
        assert_eq!((met, saw.as_str()), (false, "`false` exit 1"));
        let (met, saw) = check(
            &need("id = \"c\"\ncommand = [\"no-such-tool-x\"]"),
            dir,
            &none,
        );
        assert_eq!((met, saw.as_str()), (false, "`no-such-tool-x` not found"));
        let slow = need("id = \"c\"\ncommand = [\"sleep\", \"5\"]\ntimeout_ms = 100");
        let started = Instant::now();
        let (met, saw) = check(&slow, dir, &none);
        assert!(!met && saw.ends_with("timed out after 100 ms"), "{saw}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_missing_need_is_tried_twice_and_a_met_one_once() {
        let n = need("id = \"b\"\nenv = \"FAIRLEAD_TEST_SURELY_UNSET_X\"");
        let checked = check_all(&[n], Path::new("."));
        assert_eq!((checked[0].met, checked[0].tries), (false, 2));
        assert!(checked[0].millis >= RETRY_AFTER.as_millis());
    }
}
