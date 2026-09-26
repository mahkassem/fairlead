//! Which workspace packages a pnpm lockfile change reaches.

use std::collections::BTreeSet;

use fairlead_tests::lockfile::affected_importers;

const BASE: &str = r#"
lockfileVersion: '9.0'
settings:
  autoInstallPeers: true
importers:
  .:
    devDependencies:
      tool:
        specifier: ^1.0.0
        version: 1.0.0
  packages/a:
    dependencies:
      left:
        specifier: ^1.0.0
        version: 1.0.0
      b:
        specifier: workspace:*
        version: link:../b
  packages/b:
    dependencies:
      right:
        specifier: ^2.0.0
        version: 2.0.0
      alias:
        specifier: npm:real@3.0.0
        version: real@3.0.0
  packages/b/nested:
    dependencies: {}
  packages/c:
    dependencies:
      shared:
        specifier: ^1.0.0
        version: 1.0.0(peer@1.0.0)
packages:
  tool@1.0.0:
    resolution: {integrity: sha512-tool}
  left@1.0.0:
    resolution: {integrity: sha512-left}
  right@2.0.0:
    resolution: {integrity: sha512-right}
  deep@1.0.0:
    resolution: {integrity: sha512-deep}
  real@3.0.0:
    resolution: {integrity: sha512-real}
  shared@1.0.0:
    resolution: {integrity: sha512-shared}
  peer@1.0.0:
    resolution: {integrity: sha512-peer}
snapshots:
  tool@1.0.0: {}
  left@1.0.0: {}
  right@2.0.0:
    dependencies:
      deep: 1.0.0
  deep@1.0.0: {}
  real@3.0.0: {}
  shared@1.0.0(peer@1.0.0):
    dependencies:
      peer: 1.0.0
  peer@1.0.0: {}
"#;

fn affected(head: &str) -> Option<Vec<String>> {
    affected_importers(BASE, head).map(|s| s.into_iter().collect())
}

fn set(v: &[&str]) -> Option<Vec<String>> {
    Some(
        v.iter()
            .map(|s| s.to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    )
}

#[test]
fn nothing_changed_affects_nothing() {
    assert_eq!(affected(BASE), set(&[]));
}

#[test]
fn a_direct_dependency_bump_affects_its_importer() {
    let head = BASE.replace("sha512-left", "sha512-left-2");
    assert_eq!(affected(&head), set(&["packages/a"]));
}

#[test]
fn a_transitive_change_affects_every_importer_that_reaches_it_and_nested_ones() {
    let head = BASE.replace("sha512-deep", "sha512-deep-2");
    assert_eq!(affected(&head), set(&["packages/b", "packages/b/nested"]));
}

#[test]
fn an_aliased_dependency_is_followed() {
    let head = BASE.replace("sha512-real", "sha512-real-2");
    assert_eq!(affected(&head), set(&["packages/b", "packages/b/nested"]));
}

#[test]
fn a_peer_resolution_change_counts_as_a_change() {
    let head = BASE
        .replace("1.0.0(peer@1.0.0)", "1.0.0(peer@1.1.0)")
        .replace(
            "  peer@1.0.0:\n    resolution: {integrity: sha512-peer}",
            "  peer@1.1.0:\n    resolution: {integrity: sha512-peer}",
        )
        .replace("      peer: 1.0.0", "      peer: 1.1.0")
        .replace("  peer@1.0.0: {}", "  peer@1.1.0: {}");
    assert_eq!(affected(&head), set(&["packages/c"]));
}

#[test]
fn a_root_dependency_change_affects_the_root_and_so_everything() {
    let head = BASE.replace("sha512-tool", "sha512-tool-2");
    let got = affected(&head).unwrap();
    assert!(got.contains(&".".to_string()) && got.contains(&"packages/c".to_string()));
}

#[test]
fn a_removed_package_affects_whoever_depended_on_it_in_the_base() {
    let head = BASE
        .replace("    dependencies:\n      deep: 1.0.0\n", "")
        .replace("  deep@1.0.0: {}\n", "")
        .replace(
            "  deep@1.0.0:\n    resolution: {integrity: sha512-deep}\n",
            "",
        );
    assert_eq!(affected(&head), set(&["packages/b", "packages/b/nested"]));
}

#[test]
fn a_catalog_change_is_followed_through_the_importers() {
    let base = BASE.replace("settings:", "catalogs:\n  default:\n    left:\n      specifier: ^1.0.0\n      version: 1.0.0\nsettings:");
    let head = base.replace(
        "specifier: ^1.0.0\n      version: 1.0.0\nsettings",
        "specifier: ^1.0.1\n      version: 1.0.0\nsettings",
    );
    assert_eq!(affected_importers(&base, &head).map(|s| s.len()), Some(0));
}

#[test]
fn settings_overrides_patches_and_unknown_keys_are_not_scoped() {
    for head in [
        BASE.replace("autoInstallPeers: true", "autoInstallPeers: false"),
        BASE.replace("settings:", "overrides:\n  left: 1.0.1\nsettings:"),
        BASE.replace(
            "settings:",
            "patchedDependencies:\n  left@1.0.0: abc\nsettings:",
        ),
        BASE.replace("settings:", "pnpmfileChecksum: abc\nsettings:"),
    ] {
        assert_eq!(affected(&head), None, "{head}");
    }
}

#[test]
fn a_reference_to_an_unknown_package_or_malformed_text_is_not_scoped() {
    let dangling = BASE.replace("      deep: 1.0.0", "      deep: 9.9.9");
    assert_eq!(affected(&dangling), None);
    assert_eq!(affected("importers: [\n"), None);
    assert_eq!(
        affected(&format!("{BASE}\n---\nother: 1\n")),
        None,
        "multiple documents"
    );
    assert_eq!(
        affected(&BASE.replace("'9.0'", "'5.4'")),
        None,
        "an old format"
    );
}

#[test]
fn a_v6_lockfile_is_read_from_its_packages() {
    let v6_base = r#"
lockfileVersion: '6.0'
importers:
  packages/a:
    dependencies:
      left:
        specifier: ^1.0.0
        version: 1.0.0
  packages/b:
    dependencies:
      right:
        specifier: ^2.0.0
        version: 2.0.0
packages:
  /left@1.0.0:
    resolution: {integrity: sha512-left}
  /right@2.0.0:
    resolution: {integrity: sha512-right}
    dependencies:
      deep: 1.0.0
  /deep@1.0.0:
    resolution: {integrity: sha512-deep}
"#;
    let head = v6_base.replace("sha512-deep", "sha512-deep-2");
    let got: Vec<String> = affected_importers(v6_base, &head)
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(got, ["packages/b"]);
}
