//! What an agent should know for a change: lessons now, skills and docs
//! with K4.3 on, each picked by the same router from what the change reaches.

pub mod lesson;
pub mod route;
pub mod secrets;

use std::cmp::Reverse;
use std::path::Path;

use crate::plan_cmd::Planned;

/// What the planned change reaches, for every kind of knowledge: the changed
/// paths (a rename's old path too), the files they import within `hops`,
/// and the fallback when the plan runs everything.
pub fn reach(planned: &Planned, hops: usize) -> route::Reach {
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

/// A lesson offered for a change, and the files that couldn't be.
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
