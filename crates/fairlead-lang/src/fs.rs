//! A filesystem for the resolver that needs no install. It shows a virtual
//! `node_modules/<name>` at the root for every workspace package, pointing
//! at the package's folder, and maps build output that isn't on disk back to
//! the source it's built from. Real paths come back from `canonicalize`, so
//! the graph never stores a virtual one.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use oxc_resolver::{FileMetadata, FileSystem, FileSystemOs, ResolveError};

use crate::workspace::Package;

const SOURCE_FOR_OUTPUT: [(&str, &[&str]); 5] = [
    (".js", &[".ts", ".tsx", ".js"]),
    (".mjs", &[".mts", ".mjs"]),
    (".cjs", &[".cts", ".cjs"]),
    (".d.ts", &[".ts", ".tsx"]),
    (".jsx", &[".tsx", ".jsx"]),
];

#[derive(Debug, Clone, Default)]
pub struct WorkspaceFs {
    node_modules: PathBuf,
    packages: HashMap<String, PathBuf>,
    scopes: HashSet<String>,
    /// Absolute (outDir, rootDir) pairs.
    outputs: Vec<(PathBuf, PathBuf)>,
    /// Deleted files shown as empty files, and the directories above them.
    phantoms: HashSet<PathBuf>,
    phantom_dirs: HashSet<PathBuf>,
}

impl WorkspaceFs {
    pub fn for_workspace(root: &Path, packages: &[Package]) -> WorkspaceFs {
        let mut fs = WorkspaceFs {
            node_modules: root.join("node_modules"),
            ..WorkspaceFs::default()
        };
        for package in packages {
            let dir = root.join(&package.dir);
            if let Some((scope, _)) = package.name.split_once('/') {
                fs.scopes.insert(scope.to_string());
            }
            if let Some((out_dir, root_dir)) = &package.out_to_source {
                fs.outputs.push((dir.join(out_dir), dir.join(root_dir)));
            }
            fs.packages.insert(package.name.clone(), dir);
        }
        fs
    }

    pub fn with_phantoms(mut self, root: &Path, phantoms: &[String]) -> WorkspaceFs {
        for rel in phantoms {
            // Segment by segment, so the separators match the resolver's paths on Windows.
            let path = rel
                .split('/')
                .fold(root.to_path_buf(), |p, part| p.join(part));
            let mut dir = path.parent();
            while let Some(d) = dir.filter(|d| d.starts_with(root) && *d != root) {
                self.phantom_dirs.insert(d.to_path_buf());
                dir = d.parent();
            }
            self.phantoms.insert(path);
        }
        self
    }

    /// A phantom's metadata. Paths are compared without Windows' `\\?\`
    /// prefix, which the resolver's canonical directories carry.
    fn phantom(&self, path: &Path) -> Option<FileMetadata> {
        if self.phantoms.is_empty() {
            return None;
        }
        let path = crate::tree::plain(path);
        if self.phantoms.contains(&path) {
            return Some(FileMetadata::new(true, false, false));
        }
        self.phantom_dirs
            .contains(&path)
            .then(|| FileMetadata::new(false, true, false))
    }

    fn is_phantom_file(&self, path: &Path) -> bool {
        !self.phantoms.is_empty() && self.phantoms.contains(&crate::tree::plain(path))
    }

    /// The real path behind a virtual `node_modules/<package>/...` path.
    fn unvirtual(&self, path: &Path) -> Option<PathBuf> {
        let rest = path.strip_prefix(&self.node_modules).ok()?;
        let mut parts = rest
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned());
        let first = parts.next()?;
        let name = if first.starts_with('@') {
            format!("{first}/{}", parts.next()?)
        } else {
            first
        };
        let dir = self.packages.get(&name)?;
        Some(parts.fold(dir.clone(), |acc, part| acc.join(part)))
    }

    fn is_virtual_dir(&self, path: &Path) -> bool {
        if self.packages.is_empty() {
            return false;
        }
        if path == self.node_modules {
            return true;
        }
        path.parent() == Some(self.node_modules.as_path())
            && path
                .file_name()
                .is_some_and(|n| self.scopes.contains(n.to_string_lossy().as_ref()))
    }

    /// The source file a build output comes from, if one exists.
    pub fn source_for(&self, path: &Path) -> Option<PathBuf> {
        let (out_dir, root_dir) = self.outputs.iter().find(|(out, _)| path.starts_with(out))?;
        let rel = path
            .strip_prefix(out_dir)
            .ok()?
            .to_string_lossy()
            .into_owned();
        SOURCE_FOR_OUTPUT
            .iter()
            .filter(|(ext, _)| rel.ends_with(ext))
            .find_map(|(ext, sources)| {
                let stem = &rel[..rel.len() - ext.len()];
                sources
                    .iter()
                    .map(|src| root_dir.join(format!("{stem}{src}")))
                    .find(|p| p.is_file())
            })
    }

    /// The file behind `path`: out of the virtual `node_modules`, and from
    /// missing build output to its source.
    pub fn real(&self, path: &Path) -> PathBuf {
        let mapped = self.unvirtual(path).unwrap_or_else(|| path.to_path_buf());
        if mapped.exists() {
            return mapped;
        }
        self.source_for(&mapped).unwrap_or(mapped)
    }
}

impl FileSystem for WorkspaceFs {
    fn new() -> Self {
        WorkspaceFs::default()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        if self.is_phantom_file(path) {
            return Ok(Vec::new());
        }
        std::fs::read(self.real(path))
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        if self.is_phantom_file(path) {
            return Ok(String::new());
        }
        FileSystemOs::read_to_string(&self.real(path))
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        if self.is_virtual_dir(path) {
            return Ok(FileMetadata::new(false, true, false));
        }
        let real = self.real(path);
        // Phantoms first: a deleted path has nothing on disk to find, and how
        // the OS reports a missing file differs by platform.
        if let Some(meta) = self.phantom(&real) {
            return Ok(meta);
        }
        FileSystemOs::metadata(&real)
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        if self.is_virtual_dir(path)
            || self.unvirtual(path).is_some()
            || self.phantom(path).is_some()
        {
            return self.metadata(path);
        }
        match FileSystemOs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => self.metadata(path),
            other => other,
        }
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        FileSystemOs::read_link(&self.real(path))
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let real = self.real(path);
        if self.phantom(&real).is_some() {
            return Ok(real);
        }
        FileSystemOs::canonicalize(&real)
    }
}
