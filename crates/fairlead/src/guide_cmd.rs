//! `fairlead guide [topic]`: the short guides an agent follows, built into
//! the binary so the text is the installed release's own.

use std::process::ExitCode;

#[derive(clap::Args)]
pub struct GuideArgs {
    /// The guide to print; without it, the list of guides.
    #[arg(value_name = "TOPIC")]
    topic: Option<String>,
}

/// Each guide: its topic, one line on when to read it, and its text.
const GUIDES: &[(&str, &str, &str)] = &[
    (
        "setup",
        "set a repository up, and read the setup checklist",
        include_str!("../guides/setup.md"),
    ),
    (
        "loop",
        "make a change: the brief, the gate, the receipt and next",
        include_str!("../guides/loop.md"),
    ),
    (
        "gate",
        "the done gate: the budget, what is owed to CI, a hook that refuses an edit",
        include_str!("../guides/gate.md"),
    ),
    (
        "ci",
        "add Fairlead to CI, and tell whether a red check is yours",
        include_str!("../guides/ci.md"),
    ),
    (
        "upgrade",
        "move a repository to a new release",
        include_str!("../guides/upgrade.md"),
    ),
    (
        "troubleshoot",
        "a setup item that stays open: its cause and its fix, by item id",
        include_str!("../guides/troubleshoot.md"),
    ),
];

pub fn run(args: GuideArgs) -> ExitCode {
    match answer(args.topic.as_deref()) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(refusal) => {
            eprint!("{refusal}");
            ExitCode::from(2)
        }
    }
}

/// What to print: the list with no topic, a guide's text for a topic there
/// is, and as the error the refusal and the list for one there isn't.
fn answer(topic: Option<&str>) -> Result<String, String> {
    let Some(topic) = topic else {
        return Ok(list());
    };
    match GUIDES.iter().find(|guide| guide.0 == topic) {
        Some(guide) => Ok(guide.2.to_string()),
        None => Err(format!("guide: no guide named `{topic}`\n{}", list())),
    }
}

fn list() -> String {
    let mut out = String::from("guides: print one with `fairlead guide <topic>`\n");
    for guide in GUIDES {
        out.push_str(&format!("  {:<14}{}\n", guide.0, guide.1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_guide_is_short_and_starts_with_a_heading() {
        for guide in GUIDES {
            let (topic, summary, text) = *guide;
            assert!(!summary.is_empty(), "{topic} has no summary");
            assert!(!text.trim().is_empty(), "{topic} is empty");
            let lines = text.lines().count();
            assert!(lines < 60, "{topic} has {lines} lines");
            let first = text.lines().next().unwrap_or_default();
            assert!(first.starts_with("# "), "{topic} starts with `{first}`");
            assert!(text.ends_with('\n'), "{topic} doesn't end with a newline");
        }
    }

    #[test]
    fn no_topic_lists_every_guide_with_its_summary() {
        let listed = answer(None).unwrap();
        assert!(listed.contains("fairlead guide <topic>"));
        for guide in GUIDES {
            assert!(listed.contains(guide.0), "{} isn't listed", guide.0);
            assert!(listed.contains(guide.1), "{} has no summary line", guide.0);
        }
    }

    #[test]
    fn a_known_topic_prints_its_guide_and_nothing_else() {
        let text = answer(Some("gate")).unwrap();
        assert!(text.starts_with("# The done gate"));
        assert!(!text.contains("fairlead guide <topic>"));
    }

    #[test]
    fn an_unknown_topic_is_refused_with_the_list() {
        let refusal = answer(Some("nope")).unwrap_err();
        assert!(refusal.starts_with("guide: no guide named `nope`"));
        for guide in GUIDES {
            assert!(refusal.contains(guide.0), "{} isn't listed", guide.0);
        }
    }
}
