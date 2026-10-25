//! Reachability counts for every node at once. A node's set is itself and
//! every neighbour's set, except that a cut neighbour adds only itself: the
//! walk reaches a barrier but doesn't go past it. Sets are bitsets over the
//! marked nodes, 4,096 marks at a time, built once per strongly connected
//! component in an order where every component a node reaches comes first.
//! That's O((V + E) * M / 64) word operations for M marks, against
//! O(V * (V + E)) for a walk from every node.

const CHUNK: usize = 4096;
const UNSET: u32 = u32::MAX;

/// The strongly connected components of `adj`, leaving out edges into a
/// `cut` node, each listed after every component it reaches; and each
/// node's component.
pub fn components(adj: &[Vec<u32>], cut: &[bool]) -> (Vec<u32>, Vec<Vec<u32>>) {
    let n = adj.len();
    let (mut index, mut low) = (vec![UNSET; n], vec![0u32; n]);
    let mut on_stack = vec![false; n];
    let (mut stack, mut call) = (Vec::new(), Vec::<(u32, usize)>::new());
    let mut comp = vec![UNSET; n];
    let mut comps: Vec<Vec<u32>> = Vec::new();
    let mut next = 0u32;
    let mut enter = |v: u32, index: &mut [u32], low: &mut [u32], stack: &mut Vec<u32>| {
        index[v as usize] = next;
        low[v as usize] = next;
        next += 1;
        stack.push(v);
    };
    for root in 0..n as u32 {
        if index[root as usize] != UNSET {
            continue;
        }
        enter(root, &mut index, &mut low, &mut stack);
        on_stack[root as usize] = true;
        call.push((root, 0));
        while let Some(&(v, i)) = call.last() {
            let vu = v as usize;
            if let Some(&w) = adj[vu].get(i) {
                call.last_mut().expect("a frame").1 += 1;
                let wu = w as usize;
                if cut[wu] {
                    continue;
                }
                if index[wu] == UNSET {
                    enter(w, &mut index, &mut low, &mut stack);
                    on_stack[wu] = true;
                    call.push((w, 0));
                } else if on_stack[wu] {
                    low[vu] = low[vu].min(index[wu]);
                }
                continue;
            }
            call.pop();
            if let Some(&(u, _)) = call.last() {
                low[u as usize] = low[u as usize].min(low[vu]);
            }
            if low[vu] == index[vu] {
                let id = comps.len() as u32;
                let mut members = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w as usize] = false;
                    comp[w as usize] = id;
                    members.push(w);
                    if w == v {
                        break;
                    }
                }
                comps.push(members);
            }
        }
    }
    (comp, comps)
}

/// For each node, how many of `marks` its set holds.
pub fn counts(adj: &[Vec<u32>], cut: &[bool], marks: &[u32]) -> Vec<u32> {
    let (comp, comps) = components(adj, cut);
    let mut out = vec![0u32; adj.len()];
    let mut slot = vec![UNSET; adj.len()];
    for chunk in marks.chunks(CHUNK) {
        let words = chunk.len().div_ceil(64);
        for (i, &m) in chunk.iter().enumerate() {
            slot[m as usize] = i as u32;
        }
        let mut sets = vec![0u64; comps.len() * words];
        for (c, members) in comps.iter().enumerate() {
            let (done, rest) = sets.split_at_mut(c * words);
            let set = &mut rest[..words];
            for &u in members {
                mark(set, slot[u as usize]);
                for &v in &adj[u as usize] {
                    let d = comp[v as usize] as usize;
                    if cut[v as usize] {
                        mark(set, slot[v as usize]);
                    } else if d != c {
                        for (a, b) in set.iter_mut().zip(&done[d * words..(d + 1) * words]) {
                            *a |= b;
                        }
                    }
                }
            }
            let n: u32 = set.iter().map(|w| w.count_ones()).sum();
            for &u in members {
                out[u as usize] += n;
            }
        }
        for &m in chunk {
            slot[m as usize] = UNSET;
        }
    }
    out
}

fn mark(set: &mut [u64], slot: u32) {
    if slot != UNSET {
        set[slot as usize / 64] |= 1 << (slot % 64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same count, by a walk from every node.
    fn by_walk(adj: &[Vec<u32>], cut: &[bool], marks: &[u32]) -> Vec<u32> {
        (0..adj.len())
            .map(|start| {
                let mut seen = vec![false; adj.len()];
                let mut queue = vec![start];
                seen[start] = true;
                while let Some(u) = queue.pop() {
                    if u != start && cut[u] {
                        continue;
                    }
                    for &v in &adj[u] {
                        if !seen[v as usize] {
                            seen[v as usize] = true;
                            queue.push(v as usize);
                        }
                    }
                }
                marks.iter().filter(|&&m| seen[m as usize]).count() as u32
            })
            .collect()
    }

    #[test]
    fn a_cycle_shares_one_set_and_a_cut_node_adds_only_itself() {
        // 0 -> 1 -> 2 -> 0 is a cycle; 2 -> 3, a cut node, which leads to 4.
        let adj = vec![vec![1], vec![2], vec![0, 3], vec![4], vec![]];
        let cut = vec![false, false, false, true, false];
        let all: Vec<u32> = (0..5).collect();
        assert_eq!(counts(&adj, &cut, &all), vec![4, 4, 4, 2, 1]);
        assert_eq!(counts(&adj, &cut, &all), by_walk(&adj, &cut, &all));
    }

    #[test]
    fn counts_agree_with_a_walk_from_every_node_across_chunks() {
        // A pseudo-random graph larger than one chunk of marks, with cuts.
        let n = 5000u32;
        let mut seed = 7u64;
        let mut rand = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        let adj: Vec<Vec<u32>> = (0..n)
            .map(|_| (0..rand() % 3).map(|_| rand() % n).collect())
            .collect();
        let cut: Vec<bool> = (0..n).map(|_| rand() % 17 == 0).collect();
        let marks: Vec<u32> = (0..n).collect();
        assert_eq!(counts(&adj, &cut, &marks), by_walk(&adj, &cut, &marks));
    }
}
