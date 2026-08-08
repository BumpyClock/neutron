use super::*;
use std::fs;
use tempfile::TempDir;

const REV: &str = "1111111111111111111111111111111111111111";
const OLD_REV: &str = "2222222222222222222222222222222222222222";

fn fixture() -> (TempDir, Compatibility) {
    let directory = TempDir::new().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join("example/src")).unwrap();
    fs::write(root.join("example/src/lib.rs"), "").unwrap();
    fs::write(
        root.join("compatibility.toml"),
        format!(
            r#"
schema = 1
[framework]
name = "framework"
version = "0.7.0"
repository = "https://example.invalid/framework"
rust_msrv = "1.90"
pinned_toolchain = "1.95.0"
audit_toolchain = "1.97.1"
previous_release = "0.6.0"
previous_release_gpui_rev = "{OLD_REV}"
[gpui]
repository = "https://github.com/BumpyClock/gpui"
rev = "{REV}"
zed_repository = "https://github.com/zed-industries/zed"
zed_upstream_base = "unknown"
[[gpui.packages]]
dependency = "gpui"
registry_package = "bumpyclock-gpui"
version = "0.7.0"
crate_path = "crates/gpui"
public_api = true
features = []
registry_status = "unavailable"
registry_note = "owner approval required"
[release]
crates_io_ready = false
blockers = ["Zed upstream base must be recorded"]
[[platforms]]
name = "macOS"
target = "aarch64-apple-darwin"
build = "verified"
unit = "verified"
headless = "verified"
native_runtime = "verified"
renderer = "not-verified"
package = "not-verified"
maturity = "preview"
notes = "fixture"
"#
        ),
    )
    .unwrap();
    fs::write(
        root.join("Cargo.toml"),
        format!(
            r#"
[workspace]
members = ["example"]
[workspace.package]
version = "0.7.0"
rust-version = "1.90"
[workspace.dependencies]
gpui = {{ package = "bumpyclock-gpui", version = "=0.7.0", git = "https://github.com/BumpyClock/gpui", rev = "{REV}" }}
"#
        ),
    )
    .unwrap();
    fs::write(
        root.join("example/Cargo.toml"),
        r#"
[package]
name = "example"
version = "0.1.0"
edition = "2024"
[dependencies]
gpui.workspace = true
"#,
    )
    .unwrap();
    fs::write(
        root.join("Cargo.lock"),
        format!(
            r#"
version = 4
[[package]]
name = "bumpyclock-gpui"
version = "0.7.0"
source = "git+https://github.com/BumpyClock/gpui?rev={REV}#{REV}"
"#
        ),
    )
    .unwrap();
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.95.0\"\n",
    )
    .unwrap();
    let compatibility = load(root).unwrap();
    fs::write(root.join(GENERATED_FILE), render(&compatibility)).unwrap();
    (directory, compatibility)
}

fn errors(root: &Path, compatibility: &Compatibility) -> String {
    validate(root, compatibility, None, true).errors.join("\n")
}

#[test]
fn accepts_coherent_fixture() {
    let (directory, compatibility) = fixture();
    assert_eq!(errors(directory.path(), &compatibility), "");
}

#[test]
fn rejects_different_revision() {
    let (directory, compatibility) = fixture();
    fs::write(
        directory.path().join("example/Cargo.toml"),
        format!(
            r#"
[package]
name = "example"
version = "0.1.0"
[dependencies]
gpui = {{ package = "bumpyclock-gpui", version = "=0.7.0", git = "https://github.com/BumpyClock/gpui", rev = "{OLD_REV}" }}
"#
        ),
    )
    .unwrap();
    assert!(errors(directory.path(), &compatibility).contains("uses revision"));
}

#[test]
fn rejects_floating_branch() {
    let (directory, compatibility) = fixture();
    let path = directory.path().join("Cargo.toml");
    let source = fs::read_to_string(&path)
        .unwrap()
        .replace(&format!("rev = \"{REV}\""), "branch = \"main\"");
    fs::write(path, source).unwrap();
    let actual = errors(directory.path(), &compatibility);
    assert!(actual.contains("not branch/tag"));
    assert!(actual.contains("missing `rev`"));
}

#[test]
fn rejects_git_only_dependency() {
    let (directory, compatibility) = fixture();
    let path = directory.path().join("Cargo.toml");
    let source = fs::read_to_string(&path)
        .unwrap()
        .replace("version = \"=0.7.0\", ", "");
    fs::write(path, source).unwrap();
    assert!(errors(directory.path(), &compatibility).contains("Git-only"));
}

#[test]
fn rejects_non_exact_public_version() {
    let (directory, compatibility) = fixture();
    let path = directory.path().join("Cargo.toml");
    let source = fs::read_to_string(&path)
        .unwrap()
        .replace("version = \"=0.7.0\"", "version = \"0.7\"");
    fs::write(path, source).unwrap();
    assert!(errors(directory.path(), &compatibility).contains("is not exact"));
}

#[test]
fn rejects_git_package_version_mismatch() {
    let (directory, compatibility) = fixture();
    let path = directory.path().join("Cargo.lock");
    let source = fs::read_to_string(&path)
        .unwrap()
        .replace("version = \"0.7.0\"", "version = \"0.8.0\"");
    fs::write(path, source).unwrap();
    assert!(errors(directory.path(), &compatibility).contains("declares version"));
}

#[test]
fn rejects_any_canonical_gpui_lock_entry_at_another_revision() {
    let (directory, compatibility) = fixture();
    let path = directory.path().join("Cargo.lock");
    let mut source = fs::read_to_string(&path).unwrap();
    source.push_str(&format!(
        r#"
[[package]]
name = "engine-helper"
version = "0.1.0"
source = "git+https://github.com/BumpyClock/gpui?rev={OLD_REV}#{OLD_REV}"
"#
    ));
    fs::write(path, source).unwrap();
    assert!(
        errors(directory.path(), &compatibility)
            .contains("canonical GPUI source at revision other than")
    );
}

#[test]
fn rejects_lookalike_gpui_lock_source() {
    let (directory, compatibility) = fixture();
    let path = directory.path().join("Cargo.lock");
    let source = fs::read_to_string(&path).unwrap().replace(
        "https://github.com/BumpyClock/gpui?",
        "https://github.com/BumpyClock/gpui-mirror?",
    );
    fs::write(path, source).unwrap();
    assert!(
        errors(directory.path(), &compatibility)
            .contains("Cargo.lock must resolve exactly one Git package")
    );
}

#[test]
fn reports_root_workspace_dependency_errors_once() {
    let (directory, compatibility) = fixture();
    let path = directory.path().join("Cargo.toml");
    let source = fs::read_to_string(&path)
        .unwrap()
        .replace(&format!("rev = \"{REV}\""), &format!("rev = \"{OLD_REV}\""));
    fs::write(path, source).unwrap();
    assert_eq!(
        errors(directory.path(), &compatibility)
            .matches("uses revision")
            .count(),
        1
    );
}

#[test]
fn rejects_wrong_registry_alias() {
    let (directory, compatibility) = fixture();
    let path = directory.path().join("Cargo.toml");
    let source = fs::read_to_string(&path)
        .unwrap()
        .replace("package = \"bumpyclock-gpui\"", "package = \"wrong-gpui\"");
    fs::write(path, source).unwrap();
    assert!(errors(directory.path(), &compatibility).contains("aliases registry package"));
}

#[test]
fn accepts_implicit_package_identity_when_names_match() {
    let (_, mut compatibility) = fixture();
    compatibility.gpui.packages[0].dependency = "gpui".into();
    compatibility.gpui.packages[0].registry_package = "gpui".into();
    let dependency: Value = toml::from_str(&format!(
        r#"version = "=0.7.0"
git = "https://github.com/BumpyClock/gpui"
rev = "{REV}"
"#
    ))
    .unwrap();
    let mut actual = Vec::new();
    validate_dependency(
        "fixture",
        "gpui",
        &dependency,
        &compatibility.gpui.packages[0],
        &compatibility,
        false,
        &mut actual,
    );
    assert_eq!(actual, Vec::<String>::new());
}

#[test]
fn maps_fork_registry_status_by_explicit_package_identity() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join("fork.toml"),
        r#"
[[registry-packages]]
workspace = "gpui"
package = "bumpyclock-gpui"
status = "selected-unpublished"
"#,
    )
    .unwrap();

    let statuses = fork_registry_statuses(directory.path()).unwrap();
    assert_eq!(
        statuses.get("bumpyclock_gpui").map(String::as_str),
        Some("selected-unpublished")
    );
    assert!(!statuses.contains_key("gpui"));
}

#[test]
fn selected_unpublished_is_valid_but_fails_registry_gate() {
    let (_, mut compatibility) = fixture();
    compatibility.gpui.packages[0].registry_status = "selected-unpublished".into();
    let mut metadata_errors = Vec::new();
    validate_metadata(&compatibility, &mut metadata_errors);
    assert!(metadata_errors.is_empty(), "{metadata_errors:?}");

    let node = PlanNode {
        id: "repo/bumpyclock-gpui".into(),
        repository: "repo".into(),
        package: "gpui".into(),
        registry_package: "bumpyclock-gpui".into(),
        version: "0.7.0".into(),
        prerequisites: BTreeSet::new(),
        metadata_ready: true,
        registry_status: "selected-unpublished".into(),
        full_dry_run: "blocked: registry publication is deferred".into(),
        non_registry_blocker: false,
    };
    let error = require_registry_gate(&[node]).unwrap_err().to_string();
    assert!(error.contains("registry=selected-unpublished"));
}

#[test]
fn rejects_engine_feature_drift() {
    let (directory, compatibility) = fixture();
    let path = directory.path().join("Cargo.toml");
    let source = fs::read_to_string(&path).unwrap().replace(
        &format!("rev = \"{REV}\""),
        &format!("rev = \"{REV}\", features = [\"drift\"]"),
    );
    fs::write(path, source).unwrap();
    assert!(errors(directory.path(), &compatibility).contains("features [\"drift\"]"));
}

#[test]
fn detects_stale_generated_document() {
    let (directory, compatibility) = fixture();
    fs::write(directory.path().join(GENERATED_FILE), "stale").unwrap();
    let mut actual = Vec::new();
    validate_generated(directory.path(), &compatibility, &mut actual);
    assert_eq!(actual.len(), 1);
    assert!(actual[0].contains("is stale"));
}

#[test]
fn rejects_missing_required_metadata_field() {
    let (directory, _) = fixture();
    let path = directory.path().join(COMPATIBILITY_FILE);
    let source = fs::read_to_string(&path)
        .unwrap()
        .replace("rust_msrv = \"1.90\"\n", "");
    fs::write(path, source).unwrap();
    assert!(
        load(directory.path())
            .unwrap_err()
            .to_string()
            .contains("invalid")
    );
}

#[test]
fn rejects_invalid_platform_status() {
    let (directory, mut compatibility) = fixture();
    compatibility.platforms[0].native_runtime = "sometimes".into();
    assert!(errors(directory.path(), &compatibility).contains("invalid native_runtime status"));
}

#[test]
fn topologically_sorts_publication_prerequisites() {
    let foundation_id = "repo/foundation".to_string();
    let facade_id = "repo/facade".to_string();
    let foundation = PlanNode {
        id: foundation_id.clone(),
        repository: "repo".into(),
        package: "foundation".into(),
        registry_package: "foundation".into(),
        version: "1.0.0".into(),
        prerequisites: BTreeSet::new(),
        metadata_ready: true,
        registry_status: "unpublished".into(),
        full_dry_run: "possible".into(),
        non_registry_blocker: false,
    };
    let facade = PlanNode {
        id: facade_id.clone(),
        repository: "repo".into(),
        package: "facade".into(),
        registry_package: "facade".into(),
        version: "1.0.0".into(),
        prerequisites: BTreeSet::from([foundation_id]),
        metadata_ready: true,
        registry_status: "unpublished".into(),
        full_dry_run: "possible".into(),
        non_registry_blocker: false,
    };
    let sorted = topological_sort(BTreeMap::from([
        (facade_id, facade),
        (foundation.id.clone(), foundation),
    ]))
    .unwrap();
    assert_eq!(sorted[0].package, "foundation");
    assert_eq!(sorted[1].package, "facade");
}

#[test]
fn orders_framework_support_packages_before_facade_without_fake_dependencies() {
    let repository = "BumpyClock/gpui-component";
    let macro_id = format!("{repository}/gpui_component_macros");
    let manifest_id = format!("{repository}/gpui_component_manifest");
    let facade_id = format!("{repository}/gpui_component");
    let node = |id: String, package: &str, prerequisites: BTreeSet<String>| PlanNode {
        id,
        repository: repository.into(),
        package: package.into(),
        registry_package: package.into(),
        version: "0.7.0".into(),
        prerequisites,
        metadata_ready: true,
        registry_status: "unpublished".into(),
        full_dry_run: "possible".into(),
        non_registry_blocker: false,
    };
    let nodes = BTreeMap::from([
        (
            facade_id.clone(),
            node(
                facade_id,
                "gpui-component",
                BTreeSet::from([macro_id.clone()]),
            ),
        ),
        (
            manifest_id.clone(),
            node(manifest_id, "gpui-component-manifest", BTreeSet::new()),
        ),
        (
            macro_id.clone(),
            node(macro_id, "gpui-component-macros", BTreeSet::new()),
        ),
    ]);
    let sorted = topological_sort(nodes).unwrap();
    assert_eq!(
        sorted
            .iter()
            .map(|node| node.package.as_str())
            .collect::<Vec<_>>(),
        [
            "gpui-component-macros",
            "gpui-component-manifest",
            "gpui-component"
        ]
    );
    assert_eq!(
        sorted.last().unwrap().prerequisites,
        BTreeSet::from([format!("{repository}/gpui_component_macros")])
    );
}

#[test]
fn finds_transitive_non_dev_root_patch_reachability() {
    let package = |id: &str, name: &str, workspace: bool| CargoPackage {
        id: id.into(),
        name: name.into(),
        version: "1.0.0".into(),
        source: (!workspace).then(|| "git+https://example.invalid/patch".into()),
        publish: None,
        description: Some("fixture".into()),
        license: Some("Apache-2.0".into()),
        license_file: None,
        readme: Some("README.md".into()),
        repository: Some("https://example.invalid".into()),
        rust_version: Some("1.90".into()),
        dependencies: Vec::new(),
    };
    let dependency = |package: &str, kind: Option<&str>| CargoNodeDependency {
        pkg: package.into(),
        dep_kinds: vec![CargoNodeDependencyKind {
            kind: kind.map(str::to_owned),
        }],
    };
    let metadata = CargoMetadata {
        workspace_members: vec!["foundation".into(), "facade".into(), "dev-only".into()],
        packages: vec![
            package("foundation", "foundation", true),
            package("facade", "facade", true),
            package("dev-only", "dev-only", true),
            package("async-task", "async-task", false),
            package("calloop", "calloop", false),
        ],
        resolve: Some(CargoResolve {
            nodes: vec![
                CargoNode {
                    id: "foundation".into(),
                    deps: vec![dependency("async-task", None)],
                },
                CargoNode {
                    id: "facade".into(),
                    deps: vec![dependency("foundation", None)],
                },
                CargoNode {
                    id: "dev-only".into(),
                    deps: vec![dependency("calloop", Some("dev"))],
                },
                CargoNode {
                    id: "async-task".into(),
                    deps: Vec::new(),
                },
                CargoNode {
                    id: "calloop".into(),
                    deps: Vec::new(),
                },
            ],
        }),
    };
    let actual = root_patch_reachability(
        &metadata,
        &BTreeMap::from([
            ("async-task".into(), "async-task".into()),
            ("calloop".into(), "calloop".into()),
        ]),
    )
    .unwrap();

    assert_eq!(
        actual.get("foundation"),
        Some(&BTreeSet::from(["async-task".into()]))
    );
    assert_eq!(
        actual.get("facade"),
        Some(&BTreeSet::from(["async-task".into()]))
    );
    assert!(!actual.contains_key("dev-only"));
}

#[test]
fn require_registry_gate_rejects_non_registry_dry_run_blocker() {
    let node = PlanNode {
        id: "repo/package".into(),
        repository: "repo".into(),
        package: "package".into(),
        registry_package: "package".into(),
        version: "1.0.0".into(),
        prerequisites: BTreeSet::new(),
        metadata_ready: true,
        registry_status: "published".into(),
        full_dry_run: "blocked: non-inherited GPUI root patch: async-task".into(),
        non_registry_blocker: true,
    };

    let error = require_registry_gate(&[node]).unwrap_err().to_string();
    assert!(error.contains("async-task"));
}

#[test]
fn recognizes_only_exact_unpublished_engine_registry_failures() {
    let (_, compatibility) = fixture();
    assert!(
        unavailable_engine_registry_failure(
            "failed to select a version for the requirement `bumpyclock-gpui = \"=0.7.0\"`",
            &compatibility,
        )
        .is_some()
    );
    assert!(
        unavailable_engine_registry_failure(
            "no matching package named `bumpyclock-gpui` found",
            &compatibility,
        )
        .is_some()
    );
    assert!(
        unavailable_engine_registry_failure(
            "failed to select a version for the requirement `bumpyclock-gpui = \"=0.8.0\"`",
            &compatibility,
        )
        .is_none()
    );
    assert!(
        unavailable_engine_registry_failure(
            "failed to select a version for the requirement `serde = \"=1.0.0\"`",
            &compatibility,
        )
        .is_none()
    );
    assert!(
        unavailable_engine_registry_failure(
            "failed to select a version for the requirement `serde = \"=1.0.0\"`\nrequired by `bumpyclock-gpui = \"=0.7.0\"`",
            &compatibility,
        )
        .is_none()
    );
}

#[test]
fn registry_probe_status_is_fail_closed_for_unknown_failures() {
    assert_eq!(registry_probe_status(true, b""), "published");
    assert_eq!(
        registry_probe_status(false, b"error: could not find `fixture`"),
        "unpublished"
    );
    assert_eq!(registry_probe_status(false, b"network timeout"), "unknown");
}

#[test]
fn strict_package_commands_do_not_allow_dirty_worktrees() {
    for list in [false, true] {
        let development = cargo_package_args("fixture", list, true, true);
        assert!(development.contains(&"--allow-dirty".into()));
        assert!(development.contains(&"--no-verify".into()));

        let strict = cargo_package_args("fixture", list, false, false);
        assert!(!strict.contains(&"--allow-dirty".into()));
        assert!(!strict.contains(&"--no-verify".into()));
    }
}

#[test]
fn headless_test_command_enables_test_support() {
    assert_eq!(
        cargo_headless_test_args(),
        [
            "test",
            "--locked",
            "-p",
            "gpui-component-app",
            "--test",
            "headless",
            "--features",
            "test-support",
        ]
    );
}

#[test]
fn rejects_publishable_package_with_git_only_normal_dependency() {
    let package = CargoPackage {
        id: "published 1.0.0".into(),
        name: "published".into(),
        version: "1.0.0".into(),
        source: None,
        publish: None,
        description: Some("fixture".into()),
        license: Some("Apache-2.0".into()),
        license_file: None,
        readme: Some("README.md".into()),
        repository: Some("https://example.invalid".into()),
        rust_version: Some("1.90".into()),
        dependencies: vec![CargoDependency {
            name: "git-only".into(),
            source: Some("git+https://example.invalid/repo".into()),
            req: "*".into(),
            path: None,
            kind: None,
        }],
    };
    let metadata = CargoMetadata {
        workspace_members: vec![package.id.clone()],
        packages: vec![package],
        resolve: None,
    };
    let mut actual = Vec::new();
    validate_publishable_metadata(&metadata, &mut actual);
    assert_eq!(actual.len(), 1);
    assert!(actual[0].contains("Git-only normal dependency"));
}

fn archive_fixture(manifest: &str, include_license: bool) -> (TempDir, CargoPackage) {
    let directory = TempDir::new().unwrap();
    let root = directory.path();
    let package_root = root.join("staging/fixture-1.0.0");
    fs::create_dir_all(root.join("target/package")).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("Cargo.toml"), manifest).unwrap();
    fs::write(package_root.join("README.md"), "# fixture").unwrap();
    if include_license {
        fs::write(package_root.join("LICENSE-APACHE"), "fixture").unwrap();
    }
    let status = Command::new("tar")
        .args([
            "-czf",
            "../target/package/fixture-1.0.0.crate",
            "fixture-1.0.0",
        ])
        .current_dir(root.join("staging"))
        .status()
        .unwrap();
    assert!(status.success());
    (
        directory,
        CargoPackage {
            id: "fixture 1.0.0".into(),
            name: "fixture".into(),
            version: "1.0.0".into(),
            source: None,
            publish: None,
            description: Some("fixture".into()),
            license: Some("Apache-2.0".into()),
            license_file: None,
            readme: Some("README.md".into()),
            repository: Some("https://example.invalid".into()),
            rust_version: Some("1.90".into()),
            dependencies: Vec::new(),
        },
    )
}

#[test]
fn rejects_normalized_engine_feature_drift() {
    let (_, mut compatibility) = fixture();
    let (directory, package) = archive_fixture(
        r#"
[package]
name = "fixture"
version = "1.0.0"
[dependencies.gpui]
version = "=0.7.0"
package = "bumpyclock-gpui"
features = ["drift"]
"#,
        true,
    );
    compatibility.gpui.packages[0].features = Vec::new();
    let error = inspect_normalized_manifest(directory.path(), &package, &compatibility)
        .unwrap_err()
        .to_string();
    assert!(error.contains("normalized `gpui` features [\"drift\"]"));
}

#[test]
fn rejects_package_archive_without_license() {
    let (directory, package) = archive_fixture(
        "[package]\nname = \"fixture\"\nversion = \"1.0.0\"\n",
        false,
    );
    assert!(
        inspect_package_files(directory.path(), &package)
            .unwrap_err()
            .to_string()
            .contains("contains no LICENSE")
    );
}
