//! `fairlead import rules DIR`: path-scoped rule files from other agents
//! (Claude Code's `.claude/rules/*.md`, Cursor's `.cursor/rules/*.mdc`)
//! become SKILL.md files plus `[[skills.routes]]`, so a team starts from the
//! rules it already has. The rule files stay where they are.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fairlead_core::config::{self, LoadOptions};
use fairlead_core::pattern::Pattern;

use crate::knowledge::{lesson, short};

#[derive(clap::Subcommand)]
pub enum ImportAction {
    /// Turn path-scoped rule files (Claude Code `.md` with `paths`, Cursor `.mdc` with `globs` or `alwaysApply`) into skills plus `[[skills.routes]]`.
    Rules {
        /// The directory holding the rule files, such as `.claude/rules` or `.cursor/rules`.
        dir: PathBuf,
        /// Write the SKILL.md files and append the routes to fairlead.toml; without it, only print them.
        #[arg(long)]
        write: bool,
    },
}

/// One rule file, read as a skill and its scope.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// The rule file, as the person named it.
    pub source: String,
    pub name: String,
    pub description: String,
    pub paths: Vec<String>,
    pub always: bool,
    pub body: String,
}

impl Rule {
    /// Where its skill goes, from the repository root.
    pub fn skill_path(&self) -> String {
        format!(".claude/skills/{}/SKILL.md", self.name)
    }

    /// The SKILL.md: the open format's `name` and `description`, then the rule's body unchanged.
    pub fn skill_text(&self) -> String {
        let description = serde_json::to_string(&self.description).expect("a string prints");
        format!(
            "---\nname: {}\ndescription: {description}\n---\n{}",
            self.name, self.body
        )
    }

    /// The `[[skills.routes]]` block that scopes it.
    pub fn route(&self) -> String {
        let quote = |s: &str| toml::Value::String(s.to_string()).to_string();
        let scope = if self.always {
            "always = true".to_string()
        } else {
            let paths: Vec<String> = self.paths.iter().map(|p| quote(p)).collect();
            format!("paths = [{}]", paths.join(", "))
        };
        format!(
            "[[skills.routes]]\nskill = {}\n{scope}\n",
            quote(&self.skill_path())
        )
    }
}

/// A front matter value: one line, or a list.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    One(String),
    Many(Vec<String>),
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

/// Splits at commas outside `{...}`, so `src/**/*.{ts,tsx}` stays one glob.
fn split_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth <= 0 => {
                out.push(unquote(&s[start..i]));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(unquote(&s[start..]));
    out.retain(|p| !p.is_empty());
    out
}

/// The few keys rule files use, read line by line rather than as YAML:
/// Cursor writes globs unquoted, and YAML reads a leading `*` as an alias.
fn fields(front: &str) -> Vec<(String, Value)> {
    let lines: Vec<&str> = front.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let Some((key, rest)) = line.split_once(':').filter(|_| !indented) else {
            continue;
        };
        if key.trim_start().starts_with('#') {
            continue;
        }
        let rest = rest.trim();
        let mut nested = Vec::new();
        while i < lines.len()
            && (lines[i].starts_with(' ')
                || lines[i].starts_with('\t')
                || lines[i].starts_with('-'))
        {
            nested.push(lines[i].trim());
            i += 1;
        }
        let value = if let Some(inner) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            Value::Many(split_commas(inner))
        } else if rest.is_empty() && nested.iter().all(|l| l.starts_with('-')) {
            Value::Many(
                nested
                    .iter()
                    .map(|l| unquote(l.trim_start_matches('-')))
                    .filter(|l| !l.is_empty())
                    .collect(),
            )
        } else if rest.is_empty() || rest.starts_with('>') || rest.starts_with('|') {
            Value::One(nested.join(" "))
        } else {
            Value::One(unquote(rest))
        };
        out.push((key.trim().to_string(), value));
    }
    out
}

/// The first sentence of a body, as one line of at most 200 characters:
/// the first paragraph with markdown's markers dropped, cut at its first full stop.
fn first_sentence(body: &str) -> String {
    let paragraph: Vec<&str> = body
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(['#', '>', '-', '*', ' '])
                .trim()
        })
        .skip_while(|l| l.is_empty())
        .take_while(|l| !l.is_empty())
        .collect();
    // Emphasis markers would end up mid-sentence, and a rule that opens with
    // a bold lead such as "An endpoint." needs the next sentence to say much.
    let text = paragraph.join(" ").replace("**", "").replace("__", "");
    let mut end = 0;
    while end < text.len() {
        end = text[end..].find(". ").map_or(text.len(), |i| end + i + 1);
        if text[..end].chars().count() >= 40 {
            break;
        }
    }
    short(text[..end].trim(), 200)
}

fn as_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Many(items)) => items.clone(),
        Some(Value::One(s)) => split_commas(s),
        None => Vec::new(),
    }
}

/// A Cursor glob without a slash matches at any depth, as `**/` does here.
fn glob(raw: &str, cursor: bool) -> String {
    let g = raw.trim().trim_start_matches("./").trim_start_matches('/');
    if cursor && !g.contains('/') {
        format!("**/{g}")
    } else {
        g.to_string()
    }
}

/// Reads one rule file, or says why it's skipped. `rel` is the file's path
/// under the rules directory, which names the skill.
pub fn parse(source: &str, rel: &str, text: &str) -> Result<Rule, String> {
    let cursor = rel.ends_with(".mdc");
    let (front, body) = lesson::split(text).unwrap_or(("", text));
    let fields = fields(front);
    let get = |k: &str| fields.iter().find(|(key, _)| key == k).map(|(_, v)| v);
    let mut always =
        matches!(get("alwaysApply"), Some(Value::One(v)) if v.eq_ignore_ascii_case("true"));
    let mut paths: Vec<String> = as_list(get("paths"))
        .into_iter()
        .chain(as_list(get("globs")))
        .map(|g| glob(&g, cursor))
        .filter(|g| !g.is_empty())
        .collect();
    paths.dedup();
    if always {
        paths.clear();
    } else if paths.is_empty() && !cursor {
        // Claude Code loads a rule with no `paths` for every session, so it stays always-on.
        always = true;
    } else if paths.is_empty() {
        return Err("no scope: no `globs`, and not `alwaysApply`".into());
    }
    for p in &paths {
        Pattern::new(p).map_err(|e| format!("glob `{p}`: {e}"))?;
    }
    let stem = rel.rsplit_once('.').map_or(rel, |(s, _)| s);
    let name = lesson::slug(stem);
    if name.is_empty() {
        return Err("no name: the file name has no letters or digits".into());
    }
    let description = match get("description") {
        Some(Value::One(d)) if !d.trim().is_empty() => {
            short(&d.split_whitespace().collect::<Vec<_>>().join(" "), 200)
        }
        _ => first_sentence(body),
    };
    Ok(Rule {
        source: source.to_string(),
        name,
        description,
        paths,
        always,
        body: body.to_string(),
    })
}

/// Rule files under `dir`, sorted, as (path under `dir`, full path).
fn rule_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("md" | "mdc")
            ) {
                let rel = path.strip_prefix(dir).unwrap_or(&path);
                out.push((rel.to_string_lossy().replace('\\', "/"), path));
            }
        }
    }
    out.sort();
    out
}

/// What an import would do: the rules to write, the ones already routed,
/// and the files skipped with why.
pub struct Import {
    pub new: Vec<Rule>,
    pub routed: Vec<Rule>,
    pub skipped: Vec<(String, String)>,
}

/// Reads every rule under `dir`, setting aside skills `routed` already names.
pub fn read(dir: &Path, shown: &str, routed: &BTreeSet<String>) -> Import {
    let mut import = Import {
        new: Vec::new(),
        routed: Vec::new(),
        skipped: Vec::new(),
    };
    for (rel, path) in rule_files(dir) {
        let source = format!("{}/{rel}", shown.trim_end_matches('/'));
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| format!("can't read it: {e}"))
            .and_then(|text| parse(&source, &rel, &text));
        match parsed {
            Err(why) => import.skipped.push((source, why)),
            Ok(rule) => {
                let taken = import
                    .new
                    .iter()
                    .chain(&import.routed)
                    .find(|r| r.name == rule.name);
                if let Some(first) = taken {
                    let why = format!(
                        "the skill name `{}` is taken by {}",
                        rule.name, first.source
                    );
                    import.skipped.push((source, why));
                } else if routed.contains(&rule.skill_path()) {
                    import.routed.push(rule);
                } else {
                    import.new.push(rule);
                }
            }
        }
    }
    import
}

/// The text appended to the config: a comment naming the command, then each route.
pub fn appended(rules: &[Rule], shown: &str) -> String {
    let mut out = format!("\n# fairlead import rules {shown}\n");
    for (i, r) in rules.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&r.route());
    }
    out
}

pub fn run(action: ImportAction, cwd: &Path) -> ExitCode {
    let ImportAction::Rules { dir, write } = action;
    match rules(&dir, write, cwd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("import rules: {e}");
            ExitCode::from(2)
        }
    }
}

fn rules(dir: &Path, write: bool, cwd: &Path) -> Result<ExitCode, String> {
    let loaded =
        config::load(cwd, &LoadOptions::from_process(Vec::new())).map_err(|e| e.to_string())?;
    let root = if loaded.files.is_empty() {
        crate::graph_cmd::repo_root(cwd)
    } else {
        loaded.root.clone()
    };
    let abs = cwd.join(dir);
    if !abs.is_dir() {
        return Err(format!("{} isn't a directory", dir.display()));
    }
    let shown = dir.to_string_lossy().replace('\\', "/");
    let routed: BTreeSet<String> = loaded
        .config
        .skills
        .routes
        .items()
        .iter()
        .map(|r| r.skill.trim_start_matches("./").to_string())
        .collect();
    let import = read(&abs, &shown, &routed);
    for rule in &import.new {
        let scope = if rule.always {
            "always".to_string()
        } else {
            rule.paths.join(", ")
        };
        println!("skill {} from {} ({scope})", rule.skill_path(), rule.source);
        println!("  {}", rule.description);
    }
    for rule in &import.routed {
        println!("already routed: {} ({})", rule.skill_path(), rule.source);
    }
    for (source, why) in &import.skipped {
        println!("skipped {source}: {why}");
    }
    if import.new.is_empty() {
        println!("nothing to import from {shown}");
        return Ok(ExitCode::SUCCESS);
    }
    let text = appended(&import.new, &shown);
    if !write {
        println!("\nroutes to append to fairlead.toml:{text}");
        println!(
            "dry run: nothing written; --write writes {} skill(s) and appends the routes. The rule files stay where they are.",
            import.new.len()
        );
        return Ok(ExitCode::SUCCESS);
    }
    write_all(&root, &import.new, &text)
}

/// Writes every skill and appends the routes, or writes nothing when a
/// skill would replace a different file or the config wouldn't parse.
fn write_all(root: &Path, rules: &[Rule], text: &str) -> Result<ExitCode, String> {
    let config_path = config::find_config(root)
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| root.join("fairlead.toml"));
    if config_path.extension().and_then(|e| e.to_str()) != Some("toml") {
        return Err(format!(
            "{} isn't TOML; add the routes by hand (the dry run prints them)",
            config_path.display()
        ));
    }
    let clashes: Vec<String> = rules
        .iter()
        .filter(|r| {
            std::fs::read_to_string(root.join(r.skill_path())).is_ok_and(|t| t != r.skill_text())
        })
        .map(|r| r.skill_path())
        .collect();
    if !clashes.is_empty() {
        for c in &clashes {
            eprintln!("{c} exists with other content; move it or route it by hand");
        }
        eprintln!("nothing written");
        return Ok(ExitCode::FAILURE);
    }
    let mut before = std::fs::read_to_string(&config_path).unwrap_or_default();
    if !before.is_empty() && !before.ends_with('\n') {
        before.push('\n');
    }
    let after = format!("{before}{text}");
    toml::from_str::<toml::Table>(&after).map_err(|e| {
        format!(
            "the routes don't append cleanly to {}: {e}",
            config_path.display()
        )
    })?;
    for r in rules {
        let path = root.join(r.skill_path());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, r.skill_text()).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}", r.skill_path());
    }
    std::fs::write(&config_path, after).map_err(|e| format!("{}: {e}", config_path.display()))?;
    let name = config_path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    println!("appended {} route(s) to {name}", rules.len());
    println!("The rule files stay where they are, and their agents still load them; remove them once the skills replace them.");
    let loaded =
        config::load(root, &LoadOptions::from_process(Vec::new())).map_err(|e| e.to_string())?;
    if !loaded.problems.is_empty() {
        for p in &loaded.problems {
            eprintln!("{}: {}", p.key, p.message);
        }
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(rel: &str, text: &str) -> Result<Rule, String> {
        parse(&format!("rules/{rel}"), rel, text)
    }

    #[test]
    fn a_claude_rule_reads_paths_as_a_list_or_a_comma_string() {
        let list = rule(
            "api.md",
            "---\ndescription: API handlers\npaths:\n  - \"src/api/**/*.ts\"\n  - lib/**\n---\n# API\nBody.\n",
        )
        .unwrap();
        assert_eq!(list.name, "api");
        assert_eq!(list.paths, ["src/api/**/*.ts", "lib/**"]);
        assert_eq!(list.description, "API handlers");
        assert_eq!(list.body, "# API\nBody.\n");
        assert!(!list.always);
        let comma = rule(
            "web.md",
            "---\npaths: src/**/*.{ts,tsx}, ./docs/**\n---\nBody.\n",
        )
        .unwrap();
        assert_eq!(comma.paths, ["src/**/*.{ts,tsx}", "docs/**"]);
        let inline = rule("x.md", "---\npaths: [\"a/**\", 'b/**']\n---\nB.\n").unwrap();
        assert_eq!(inline.paths, ["a/**", "b/**"]);
    }

    #[test]
    fn a_cursor_rule_reads_globs_and_always_apply() {
        let globs = rule(
            "react.mdc",
            "---\ndescription: React components\nglobs: *.tsx, src/ui/**\nalwaysApply: false\n---\nUse hooks.\n",
        )
        .unwrap();
        assert_eq!(
            globs.paths,
            ["**/*.tsx", "src/ui/**"],
            "a slashless glob matches at any depth"
        );
        assert!(!globs.always);
        let listed = rule("l.mdc", "---\nglobs:\n  - src/**\n---\nB.\n").unwrap();
        assert_eq!(listed.paths, ["src/**"]);
        let always = rule(
            "style.mdc",
            "---\ndescription:\nglobs:\nalwaysApply: true\n---\nTabs. Always tabs.\n",
        )
        .unwrap();
        assert!(always.always);
        assert!(always.paths.is_empty());
        assert_eq!(
            always.description, "Tabs. Always tabs.",
            "sentences until it says enough"
        );
    }

    #[test]
    fn a_claude_rule_without_paths_stays_always_and_a_cursor_one_is_skipped() {
        let none = rule("notes.md", "---\ndescription: Notes\n---\nB.\n").unwrap();
        assert!(none.always && none.paths.is_empty());
        let bare = rule("plain.md", "Just text.\n").unwrap();
        assert!(bare.always);
        let off = rule("off.mdc", "---\nalwaysApply: false\n---\nB.\n").unwrap_err();
        assert!(off.starts_with("no scope"), "{off}");
    }

    #[test]
    fn a_bold_lead_is_unwrapped_and_too_short_a_sentence_takes_the_next() {
        let r = rule(
            "api.md",
            "---\npaths: [\"src/**\"]\n---\n\n**An endpoint.** Define the group in one file. Then more.\n",
        )
        .unwrap();
        assert_eq!(r.description, "An endpoint. Define the group in one file.");
    }

    #[test]
    fn the_description_falls_back_to_one_line_of_the_body() {
        let long = format!("{} end.", "word ".repeat(80));
        let r = rule(
            "n/deep.md",
            &format!("---\npaths: src/**\n---\n\n## Heading line\nmore of it. Second.\n\n{long}\n"),
        )
        .unwrap();
        assert_eq!(r.name, "n-deep", "a nested file's directory joins its name");
        assert_eq!(r.description, "Heading line more of it. Second.");
        let r = rule("long.md", &format!("---\npaths: src/**\n---\n{long}\n")).unwrap();
        assert!(r.description.chars().count() <= 201, "{}", r.description);
        assert!(!r.description.contains('\n'));
    }

    #[test]
    fn the_skill_and_route_render_as_their_formats_expect() {
        let r = rule(
            "api.md",
            "---\ndescription: Say \"hi\"\npaths: src/**\n---\nBody\n",
        )
        .unwrap();
        assert_eq!(
            r.skill_text(),
            "---\nname: api\ndescription: \"Say \\\"hi\\\"\"\n---\nBody\n"
        );
        assert_eq!(
            r.route(),
            "[[skills.routes]]\nskill = \".claude/skills/api/SKILL.md\"\npaths = [\"src/**\"]\n"
        );
    }
}
