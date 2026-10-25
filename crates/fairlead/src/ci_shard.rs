//! `ci plan --shards N` and `ci run --shard K/N`: one plan split across a
//! matrix of N jobs, each running one list, the first also running every
//! invocation that isn't split, so each runs once across the matrix.

use std::path::{Path, PathBuf};

use fairlead_core::plan::{Plan, ShardWeight};
use fairlead_tests::durations::Durations;

/// Splits the plan in place and says how.
pub fn plan(
    cwd: &Path,
    planned: &mut crate::plan_cmd::Planned,
    count: u32,
    files: &[PathBuf],
) -> Result<String, String> {
    let durations = if files.is_empty() {
        None
    } else {
        let mut d = Durations::default();
        for path in files {
            let text = std::fs::read_to_string(cwd.join(path))
                .map_err(|e| format!("could not read {}: {e}", path.display()))?;
            d.add(&planned.scan.tree.root, &path.display().to_string(), &text)?;
        }
        Some(d)
    };
    let split = fairlead_tests::shard::split(
        &planned.plan,
        &planned.scan,
        &planned.config,
        count,
        durations.as_ref(),
    )?;
    let by = match split.shards.by {
        ShardWeight::Durations => format!(
            "by durations, {} of {} files timed",
            split.timed, split.files
        ),
        ShardWeight::Files => "by file count".to_string(),
    };
    let weights: Vec<String> = split
        .shards
        .lists
        .iter()
        .map(|s| format!("{}", s.weight))
        .collect();
    let line = format!(
        "shards: {count} {by}, weights {}; split: {}",
        weights.join(" / "),
        if split.shards.split.is_empty() {
            "nothing, so shard 1 runs it all".to_string()
        } else {
            split.shards.split.join(", ")
        }
    );
    planned.plan.shards = Some(split.shards);
    Ok(line)
}

/// `K/N`, numbered from 1.
fn parse(arg: &str) -> Result<(u32, u32), String> {
    let bad = || format!("--shard {arg}: write it as K/N, such as 2/4");
    let (k, n) = arg.split_once('/').ok_or_else(bad)?;
    let k: u32 = k.trim().parse().map_err(|_| bad())?;
    let n: u32 = n.trim().parse().map_err(|_| bad())?;
    if k == 0 || k > n {
        return Err(format!("--shard {arg}: K runs from 1 to {n}"));
    }
    Ok((k, n))
}

/// Narrows the plan to shard K's invocations, and says what that is.
pub fn pick(plan: &mut Plan, arg: &str) -> Result<String, String> {
    let (k, n) = parse(arg)?;
    let shards = plan.shards.as_ref().ok_or_else(|| {
        format!("--shard {arg}: the plan has no shards; make it with `ci plan --shards {n}`")
    })?;
    if shards.count != n {
        return Err(format!(
            "--shard {arg}: the plan has {} shards, not {n}; the matrix and `ci plan --shards` must agree",
            shards.count
        ));
    }
    let shard = &shards.lists[(k - 1) as usize];
    let line = format!(
        "fairlead: shard {k} of {n}: {} invocations, {} split test files{}",
        shard.invocations.len(),
        shard.files.len(),
        if k == 1 {
            ", and everything not split"
        } else {
            ""
        }
    );
    plan.invocations = shard.invocations.clone();
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shard_is_k_of_n_from_one() {
        assert_eq!(parse("2/4"), Ok((2, 4)));
        assert_eq!(parse("1/1"), Ok((1, 1)));
        assert!(parse("0/4").is_err());
        assert!(parse("5/4").is_err());
        assert!(parse("2").is_err());
        assert!(parse("a/4").is_err());
    }
}
