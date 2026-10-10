//! `ci.escapes = "file"`: each test the merge's plan left out becomes a
//! GitHub issue a person sees. Filing is capped per run and keyed on the
//! runner and test, so an escape that keeps happening is one issue with at
//! most a note a day, not a flood.

use serde_json::{json, Value};

use crate::ci_judge::Verdict;
use fairlead_replay::github::{Curl, Reply};

pub const LABEL: &str = "fairlead-escape";
/// New issues one run may open; the rest are counted, not filed.
pub const CAP: usize = 5;
const NOTE: &str = "<!-- fairlead-escape-note -->";

pub trait Api {
    fn send(&self, method: &str, path: &str, body: &Value) -> Result<Reply<Value>, String>;
}

impl Api for Curl {
    fn send(&self, method: &str, path: &str, body: &Value) -> Result<Reply<Value>, String> {
        Curl::send(self, method, path, body)
    }
}

/// Where a filing points: the merge commit and the run that found it.
pub struct Run {
    pub repo: String,
    pub sha: String,
    pub url: Option<String>,
    /// A day ago, as the API's timestamps: the window for one note.
    pub day_ago: String,
}

fn marker(v: &Verdict) -> String {
    format!(
        "<!-- fairlead-escape runner={} test={} -->",
        v.runner, v.test
    )
}

fn ok(reply: &Reply<Value>, what: &str) -> Result<(), String> {
    if (200..300).contains(&reply.status) {
        Ok(())
    } else {
        Err(format!(
            "{what} answered {}: {}",
            reply.status, reply.message
        ))
    }
}

/// What happened to each escape, one line each.
pub fn file(api: &dyn Api, run: &Run, verdicts: &[Verdict]) -> Result<Vec<String>, String> {
    let mut escapes: Vec<&Verdict> = Vec::new();
    for v in verdicts.iter().filter(|v| v.fix.is_some()) {
        if !escapes
            .iter()
            .any(|e| e.runner == v.runner && e.test == v.test)
        {
            escapes.push(v);
        }
    }
    if escapes.is_empty() {
        return Ok(Vec::new());
    }
    let listed = api.send(
        "GET",
        &format!(
            "/repos/{}/issues?labels={LABEL}&state=all&per_page=100",
            run.repo
        ),
        &Value::Null,
    )?;
    ok(&listed, "listing escape issues")?;
    let issues = listed.body.as_array().cloned().unwrap_or_default();
    let mut lines = Vec::new();
    let mut opened = 0;
    for v in escapes {
        let mark = marker(v);
        let found = issues
            .iter()
            .filter(|i| i["body"].as_str().is_some_and(|b| b.contains(&mark)))
            .max_by_key(|i| i["number"].as_u64());
        let open = found.filter(|i| i["state"] == "open");
        if let Some(issue) = open {
            let n = issue["number"].as_u64().unwrap_or(0);
            lines.push(note(api, run, n, v)?);
            continue;
        }
        if opened == CAP {
            lines.push(format!(
                "{}: not filed, {CAP} new issues already this run",
                v.test
            ));
            continue;
        }
        let again = found.and_then(|i| i["number"].as_u64());
        let body = issue_body(run, v, &mark, again);
        let reply = api.send(
            "POST",
            &format!("/repos/{}/issues", run.repo),
            &json!({
                "title": format!("Escape: {} wasn't in the merge's plan", v.test),
                "body": body,
                "labels": [LABEL],
            }),
        )?;
        ok(&reply, "filing an escape")?;
        opened += 1;
        lines.push(format!(
            "{}: filed #{}",
            v.test,
            reply.body["number"].as_u64().unwrap_or(0)
        ));
    }
    Ok(lines)
}

fn issue_body(run: &Run, v: &Verdict, mark: &str, again: Option<u64>) -> String {
    let mut body = format!(
        "{mark}\nThe merge's plan left out `{}` (runner `{}`), and it failed on {}.\n\n",
        v.test, v.runner, run.sha
    );
    if let Some(url) = &run.url {
        body.push_str(&format!("Found by {url}.\n\n"));
    }
    if let Some(n) = again {
        body.push_str(&format!("It escaped before in #{n}, which is closed.\n\n"));
    }
    body.push_str(&format!(
        "A rule that would have selected it:\n\n```toml\n{}\n```\n\nFiled by `fairlead ci run --judge` because `ci.escapes = \"file\"`.\n",
        v.fix.as_deref().unwrap_or_default()
    ));
    body
}

/// A note on the open issue, unless one was left in the last day.
fn note(api: &dyn Api, run: &Run, n: u64, v: &Verdict) -> Result<String, String> {
    let recent = api.send(
        "GET",
        &format!(
            "/repos/{}/issues/{n}/comments?since={}&per_page=100",
            run.repo, run.day_ago
        ),
        &Value::Null,
    )?;
    ok(&recent, "reading the escape's notes")?;
    let noted = recent.body.as_array().into_iter().flatten().any(|c| {
        c["body"].as_str().is_some_and(|b| b.contains(NOTE))
            && c["created_at"]
                .as_str()
                .is_some_and(|t| t >= run.day_ago.as_str())
    });
    if noted {
        return Ok(format!(
            "{}: escaped again; #{n} already has today's note",
            v.test
        ));
    }
    let mut body = format!("{NOTE}\nEscaped again on {}", run.sha);
    if let Some(url) = &run.url {
        body.push_str(&format!(", found by {url}"));
    }
    body.push('.');
    let reply = api.send(
        "POST",
        &format!("/repos/{}/issues/{n}/comments", run.repo),
        &json!({ "body": body }),
    )?;
    ok(&reply, "noting a repeat escape")?;
    Ok(format!("{}: escaped again; noted on #{n}", v.test))
}

/// The run from the Actions environment, or why it can't file.
pub fn run_from_env() -> Result<Run, String> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let repo = var("GITHUB_REPOSITORY").ok_or("GITHUB_REPOSITORY isn't set")?;
    let sha = var("GITHUB_SHA").unwrap_or_else(|| "this commit".into());
    let url = match (var("GITHUB_SERVER_URL"), var("GITHUB_RUN_ID")) {
        (Some(server), Some(id)) => Some(format!("{server}/{repo}/actions/runs/{id}")),
        _ => None,
    };
    let now = std::time::SystemTime::now();
    let day = std::time::Duration::from_secs(86_400);
    let stamp = |t| {
        let s = fairlead_guard::events::timestamp(t);
        format!("{}Z", &s[..19])
    };
    Ok(Run {
        repo,
        sha,
        url,
        day_ago: stamp(now - day),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// GitHub as a list of issues and comments, recording every write.
    struct Fake {
        issues: Value,
        comments: Value,
        writes: RefCell<Vec<(String, String, Value)>>,
    }

    impl Api for Fake {
        fn send(&self, method: &str, path: &str, body: &Value) -> Result<Reply<Value>, String> {
            if method == "GET" {
                let body = if path.contains("/comments") {
                    &self.comments
                } else {
                    &self.issues
                };
                return Ok(Reply::new(200, body.clone()));
            }
            self.writes
                .borrow_mut()
                .push((method.into(), path.into(), body.clone()));
            Ok(Reply::new(
                201,
                json!({ "number": 90 + self.writes.borrow().len() }),
            ))
        }
    }

    fn escaped(test: &str) -> Verdict {
        Verdict {
            runner: "vitest".into(),
            test: test.into(),
            verdict: "escaped".into(),
            fix: Some("[[tests.owners]] match = \"test/**\", covers = [\"src/**\"]".into()),
        }
    }

    fn run() -> Run {
        Run {
            repo: "o/r".into(),
            sha: "abc".into(),
            url: Some("https://github.com/o/r/actions/runs/1".into()),
            day_ago: "2026-10-09T12:00:00Z".into(),
        }
    }

    fn fake(issues: Value, comments: Value) -> Fake {
        Fake {
            issues,
            comments,
            writes: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn a_new_escape_is_one_labelled_issue_and_a_flood_stops_at_the_cap() {
        let api = fake(json!([]), json!([]));
        let verdicts: Vec<Verdict> = (0..7)
            .map(|i| escaped(&format!("test/t{i}.test.ts")))
            .collect();
        let lines = file(&api, &run(), &verdicts).unwrap();
        let writes = api.writes.borrow();
        assert_eq!(writes.len(), CAP);
        assert_eq!(writes[0].2["labels"], json!([LABEL]));
        assert!(writes[0].2["body"]
            .as_str()
            .unwrap()
            .starts_with("<!-- fairlead-escape runner=vitest test=test/t0.test.ts -->"));
        assert_eq!(lines.iter().filter(|l| l.contains("not filed")).count(), 2);
    }

    #[test]
    fn a_repeat_on_an_open_issue_is_a_note_at_most_once_a_day() {
        let open =
            json!([{ "number": 7, "state": "open", "body": marker(&escaped("test/a.test.ts")) }]);
        let api = fake(open.clone(), json!([]));
        let lines = file(
            &api,
            &run(),
            &[escaped("test/a.test.ts"), escaped("test/a.test.ts")],
        )
        .unwrap();
        assert_eq!(lines, ["test/a.test.ts: escaped again; noted on #7"]);
        assert_eq!(api.writes.borrow()[0].1, "/repos/o/r/issues/7/comments");
        let today = json!([{ "body": format!("{NOTE}\nEscaped again"), "created_at": "2026-10-10T08:00:00Z" }]);
        let api = fake(open, today);
        file(&api, &run(), &[escaped("test/a.test.ts")]).unwrap();
        assert!(
            api.writes.borrow().is_empty(),
            "no second note the same day"
        );
    }

    #[test]
    fn an_escape_after_its_issue_closed_files_a_new_one_that_names_the_old() {
        let closed =
            json!([{ "number": 3, "state": "closed", "body": marker(&escaped("test/a.test.ts")) }]);
        let api = fake(closed, json!([]));
        file(&api, &run(), &[escaped("test/a.test.ts")]).unwrap();
        let writes = api.writes.borrow();
        assert_eq!(writes[0].1, "/repos/o/r/issues");
        assert!(writes[0].2["body"]
            .as_str()
            .unwrap()
            .contains("escaped before in #3, which is closed"));
    }
}
