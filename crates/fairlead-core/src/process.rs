//! Starting a command the config names, found the way a shell finds it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The extensions a Windows shell can start; PATHEXT may list more.
const RUNNABLE: [&str; 4] = [".com", ".exe", ".bat", ".cmd"];

/// A `Command` for a config's program, to run in `cwd`. On Windows,
/// `Command` looks a bare name up with only `.exe` appended, so it can't
/// start the `.cmd` shims npm, pnpm and yarn install; the name is looked up
/// with each of PATHEXT's extensions instead, as a shell does.
pub fn command(program: &str, cwd: &Path) -> Command {
    let found = if cfg!(windows) {
        find(
            program,
            cwd,
            std::env::var_os("PATH"),
            std::env::var_os("PATHEXT"),
        )
    } else {
        None
    };
    let mut command = Command::new(found.unwrap_or_else(|| PathBuf::from(program)));
    command.current_dir(cwd);
    command
}

/// The file a shell would start for a program with no extension: each PATH
/// directory in turn, or `cwd` for a relative path such as
/// `node_modules/.bin/eslint`, tried with each runnable PATHEXT extension.
fn find(
    program: &str,
    cwd: &Path,
    path: Option<OsString>,
    pathext: Option<OsString>,
) -> Option<PathBuf> {
    let given = Path::new(program);
    if program.is_empty() || given.extension().is_some() {
        return None;
    }
    let listed: Vec<String> = pathext
        .map(|p| {
            p.to_string_lossy()
                .split(';')
                .map(|e| e.trim().to_ascii_lowercase())
                .filter(|e| RUNNABLE.contains(&e.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let exts = if listed.is_empty() {
        RUNNABLE.iter().map(|e| e.to_string()).collect()
    } else {
        listed
    };
    let dirs: Vec<PathBuf> = if given.is_absolute() {
        vec![PathBuf::new()]
    } else if given.components().count() > 1 {
        vec![cwd.to_path_buf()]
    } else {
        std::env::split_paths(&path?).collect()
    };
    dirs.iter()
        .flat_map(|dir| {
            exts.iter()
                .map(move |ext| dir.join(format!("{program}{ext}")))
        })
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_name_finds_a_script_shim_in_path_order_and_a_relative_one_beside_cwd() {
        let dir = std::env::temp_dir().join(format!("fairlead-process-{}", std::process::id()));
        let (first, second) = (dir.join("first"), dir.join("second"));
        std::fs::create_dir_all(second.join("node_modules/.bin")).unwrap();
        std::fs::create_dir_all(&first).unwrap();
        std::fs::write(first.join("tool.cmd"), "").unwrap();
        std::fs::write(second.join("tool.exe"), "").unwrap();
        std::fs::write(second.join("node_modules/.bin/lint.cmd"), "").unwrap();
        let path = std::env::join_paths([&first, &second]).ok();
        let ext = Some(OsString::from(".COM;.EXE;.BAT;.CMD;.VBS"));
        assert_eq!(
            find("tool", &dir, path.clone(), ext.clone()),
            Some(first.join("tool.cmd"))
        );
        assert_eq!(
            find("node_modules/.bin/lint", &second, None, ext.clone()),
            Some(second.join("node_modules/.bin/lint.cmd"))
        );
        assert_eq!(find("tool.exe", &dir, path.clone(), ext.clone()), None);
        assert_eq!(find("missing", &dir, path, ext), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
