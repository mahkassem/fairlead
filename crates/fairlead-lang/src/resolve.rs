//! Resolving specifiers with `oxc_resolver` over the workspace filesystem,
//! and sorting the results into what the graph needs.

use std::collections::HashSet;
use std::path::Path;

use fairlead_core::config::Graph as GraphConfig;
use oxc_resolver::{
    ResolveOptions, ResolverGeneric, TsconfigDiscovery, TsconfigOptions, TsconfigReferences,
};

use crate::fs::WorkspaceFs;
use crate::tree::Tree;
use crate::workspace::Package;

const EXTENSIONS: [&str; 9] = [
    ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A file in the repository, repo-relative.
    File(String),
    /// A workspace package whose target isn't on disk (build output, say):
    /// the edge goes to the whole package.
    Package(String),
    /// A package outside the repository, or a runtime builtin.
    External,
    /// A bare specifier that didn't resolve and names no workspace package:
    /// usually a package that isn't installed, but possibly an alias.
    NotFound,
    /// Couldn't be resolved; the planner widens around it.
    Unresolved,
}

pub struct Resolver {
    fs: WorkspaceFs,
    with_tsconfig: ResolverGeneric<WorkspaceFs>,
    plain: ResolverGeneric<WorkspaceFs>,
    packages: HashSet<String>,
    /// Package folders with a trailing `/`, and their names.
    dirs: Vec<(String, String)>,
}

fn options(config: &GraphConfig, root: &Path, tsconfig: bool) -> ResolveOptions {
    let alias =
        |from: &str, to: &[&str]| (from.to_string(), to.iter().map(|s| s.to_string()).collect());
    ResolveOptions {
        extensions: EXTENSIONS.iter().map(|s| s.to_string()).collect(),
        extension_alias: vec![
            alias(".js", &[".ts", ".tsx", ".js", ".jsx"]),
            alias(".mjs", &[".mts", ".mjs"]),
            alias(".cjs", &[".cts", ".cjs"]),
        ],
        condition_names: config.conditions.items().to_vec(),
        main_fields: vec!["module".into(), "main".into()],
        tsconfig: tsconfig.then(|| match config.tsconfig.as_str() {
            "auto" => TsconfigDiscovery::Auto,
            path => TsconfigDiscovery::Manual(TsconfigOptions {
                config_file: root.join(path),
                references: TsconfigReferences::Auto,
            }),
        }),
        ..ResolveOptions::default()
    }
}

impl Resolver {
    pub fn new(root: &Path, packages: &[Package], config: &GraphConfig) -> Resolver {
        Resolver::over(
            WorkspaceFs::for_workspace(root, packages),
            root,
            packages,
            config,
        )
    }

    /// A resolver that also sees `phantoms`, repo-relative files that no
    /// longer exist, as empty files.
    pub fn with_phantoms(
        root: &Path,
        packages: &[Package],
        config: &GraphConfig,
        phantoms: &[String],
    ) -> Resolver {
        let fs = WorkspaceFs::for_workspace(root, packages).with_phantoms(root, phantoms);
        Resolver::over(fs, root, packages, config)
    }

    fn over(fs: WorkspaceFs, root: &Path, packages: &[Package], config: &GraphConfig) -> Resolver {
        Resolver {
            with_tsconfig: ResolverGeneric::new_with_file_system(
                fs.clone(),
                options(config, root, true),
            ),
            plain: ResolverGeneric::new_with_file_system(fs.clone(), options(config, root, false)),
            fs,
            packages: packages.iter().map(|p| p.name.clone()).collect(),
            dirs: packages
                .iter()
                .map(|p| (format!("{}/", p.dir), p.name.clone()))
                .collect(),
        }
    }

    /// What `spec` in `file` resolves to, and whether the file's tsconfig
    /// had to be skipped to get there.
    pub fn resolve(&self, tree: &Tree, file: &str, spec: &str) -> (Target, bool) {
        let abs = tree.abs(file);
        let (result, fell_back) = match self.with_tsconfig.resolve_file(&abs, spec) {
            Ok(r) => (Ok(r), false),
            Err(first) => match self.plain.resolve_file(&abs, spec) {
                Ok(r) => (Ok(r), true),
                Err(_) => (Err(first), false),
            },
        };
        let target = match result {
            // The resolver canonicalizes a package's folder, not the file in it,
            // so build output in a workspace package is mapped here.
            Ok(resolution) => {
                let real = self.fs.real(resolution.path());
                match tree.rel(&real) {
                    Some(rel) if rel.split('/').any(|p| p == "node_modules") => Target::External,
                    Some(rel) if tree.contains(&rel) => Target::File(rel),
                    Some(rel) => self.ignored(tree, &real, &rel),
                    None => Target::External,
                }
            }
            Err(_) => self.unresolved(spec),
        };
        (target, fell_back)
    }

    /// A resolved file git ignores, such as local build output: its source
    /// when the package's tsconfig says where that is, else its package.
    fn ignored(&self, tree: &Tree, real: &Path, rel: &str) -> Target {
        if let Some(source) = self.fs.source_for(real).and_then(|s| tree.rel(&s)) {
            if tree.contains(&source) {
                return Target::File(source);
            }
        }
        self.dirs
            .iter()
            .filter(|(dir, _)| rel.starts_with(dir.as_str()))
            .max_by_key(|(dir, _)| dir.len())
            .map_or(Target::Unresolved, |(_, name)| {
                Target::Package(name.clone())
            })
    }

    /// Where `spec` in `file` lands, repo-relative, whether or not that file
    /// is in the tree: how a deleted file's importers are found again.
    pub fn resolve_path(&self, tree: &Tree, file: &str, spec: &str) -> Option<String> {
        let abs = tree.abs(file);
        let resolution = self
            .with_tsconfig
            .resolve_file(&abs, spec)
            .or_else(|_| self.plain.resolve_file(&abs, spec))
            .ok()?;
        tree.rel(&self.fs.real(resolution.path()))
    }

    fn unresolved(&self, spec: &str) -> Target {
        if spec.starts_with('.') || spec.starts_with('/') || spec.starts_with('#') {
            return Target::Unresolved;
        }
        match package_name(spec) {
            Some(name) if self.packages.contains(name) => Target::Package(name.to_string()),
            Some(_) => Target::NotFound,
            None => Target::Unresolved,
        }
    }
}

/// The package a bare specifier names: `a` or `@scope/a`. `None` when it
/// can't be a package name, such as a path alias like `@/lib` or `~/x`.
pub fn package_name(spec: &str) -> Option<&str> {
    let valid = |s: &str| {
        !s.is_empty()
            && !s.starts_with('.')
            && !s.starts_with('_')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._:".contains(c))
    };
    if let Some(scoped) = spec.strip_prefix('@') {
        let (scope, rest) = scoped.split_once('/')?;
        let name = rest.split('/').next()?;
        (valid(scope) && valid(name)).then(|| &spec[..1 + scope.len() + 1 + name.len()])
    } else {
        let name = spec.split('/').next()?;
        valid(name).then_some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_from_bare_specifiers() {
        assert_eq!(package_name("lodash/fp"), Some("lodash"));
        assert_eq!(package_name("@scope/pkg/sub/path"), Some("@scope/pkg"));
        assert_eq!(package_name("node:fs"), Some("node:fs"));
        assert_eq!(package_name("@/lib/utils"), None);
        assert_eq!(package_name("~/x"), None);
    }
}
