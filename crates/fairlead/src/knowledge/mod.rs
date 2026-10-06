//! What an agent should know for a change: lessons now, skills and docs
//! with K4.3 on, each picked by the same router from what the change reaches.

pub mod lesson;
pub mod route;
pub mod secrets;
pub mod skill;
pub mod sync;

use std::cmp::Reverse;
use std::path::Path;

use crate::plan_cmd::Planned;

/// What the planned change reaches, for every kind of knowledge: the changed
/// paths (a rename's old path too), the files they import within `hops`,
/// and the fallback when the plan runs everything.
pub fn reach(planned: &Planned) -> route::Reach {
    let hops = route::Hops {
        imports: planned.config.skills.imports,
        importers: planned.config.skills.importers,
    };
    let scan = &planned.scan;
    let modules = fairlead_tests::modules::Modules::discover(
        &scan.tree,
        &scan.packages,
        &planned.config.modules,
    )
    .unwrap_or_default();
    let mut changed: Vec<String> = Vec::new();
    for c in &planned.plan.changed {
        changed.push(c.path.clone());
        changed.extend(c.from.clone());
    }
    let module_of = |p: &str| modules.name_of(p).map(str::to_string);
    route::Reach::new(&changed, &scan.graph, hops, &module_of, planned.plan.all)
}

/// What a change was offered, as (name, why), and the files that couldn't be read.
pub struct Offered {
    pub lessons: Vec<(String, String)>,
    pub bad: Vec<lesson::Bad>,
}

/// The lessons a change should know, named first, then used, always and
/// fallback; enforced ones first within each, then the newest.
pub fn lessons(planned: &Planned, root: &Path, reach: &route::Reach) -> Offered {
    let (all, bad) = lesson::load(root, &planned.config.memory);
    let today = lesson::today();
    let picked = route::select(
        &all,
        |l| &l.scope,
        |l| {
            (
                l.front.check.is_none(),
                Reverse(l.front.added.clone()),
                l.front.id.clone(),
            )
        },
        reach,
    );
    let lessons = picked
        .into_iter()
        .map(|(i, reason)| {
            let l = &all[i];
            let due = if l.due(&today) {
                "; due for review"
            } else {
                ""
            };
            (
                l.front.id.clone(),
                format!("{} ({reason}{due})", l.front.title),
            )
        })
        .collect();
    Offered { lessons, bad }
}

/// The routed skills a change should load, in the brief's order: named,
/// used, always, fallback, then by name.
pub fn skills(planned: &Planned, root: &Path, reach: &route::Reach) -> Offered {
    let (all, bad) = skill::load(root, &planned.config.skills);
    let picked = route::select(&all, |s| &s.scope, |s| s.name.clone(), reach);
    let skills = picked
        .into_iter()
        .map(|(i, reason)| {
            let s = &all[i];
            let what = if s.description.is_empty() {
                s.path.clone()
            } else {
                short(&s.description, 70)
            };
            (s.name.clone(), format!("{what} ({reason})"))
        })
        .collect();
    Offered {
        lessons: skills,
        bad,
    }
}

/// At most `n` characters, cut at a word with "…" when it's longer.
fn short(text: &str, n: usize) -> String {
    if text.chars().count() <= n {
        return text.to_string();
    }
    let cut: String = text.chars().take(n).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{cut}…")
}
