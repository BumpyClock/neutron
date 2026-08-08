#![allow(
    clippy::disallowed_methods,
    reason = "xtask runs synchronous Cargo and tar subprocesses"
)]

use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const VALID_STATUSES: &[&str] = &[
    "upstream-candidate",
    "submitted-upstream",
    "accepted-upstream",
    "temporary",
    "fork-only",
    "obsolete",
];
const VALID_REGISTRY_STATUSES: &[&str] = &[
    "conflict",
    "unavailable",
    "owner-decision-required",
    "selected-unpublished",
    "controlled",
    "published",
];
const SOURCE_GATES: &[(&str, &[&str])] = &[
    (
        "workspace all-target check",
        &["check", "--locked", "--workspace", "--all-targets"],
    ),
    ("scheduler tests", &["test", "--locked", "-p", "scheduler"]),
    (
        "gpui library tests",
        &[
            "test",
            "--locked",
            "-p",
            "bumpyclock-gpui",
            "--features",
            "bench,profiler",
            "--lib",
        ],
    ),
    (
        "gpui wgpu library tests",
        &["test", "--locked", "-p", "gpui_wgpu", "--lib"],
    ),
    (
        "gpui examples",
        &["check", "--locked", "-p", "bumpyclock-gpui", "--examples"],
    ),
];

#[derive(Debug, Deserialize)]
struct ForkMetadata {
    #[serde(rename = "schema-version")]
    schema_version: u64,
    #[serde(rename = "upstream-repository")]
    upstream_repository: String,
    #[serde(rename = "upstream-base-commit")]
    upstream_base_commit: String,
    #[serde(rename = "last-synchronization-date")]
    last_synchronization_date: String,
    #[serde(rename = "fork-repository")]
    fork_repository: String,
    #[serde(rename = "patch-clusters")]
    patch_clusters: Vec<PatchCluster>,
    #[serde(default, rename = "registry-packages")]
    registry_packages: Vec<RegistryPackage>,
}

#[derive(Debug, Deserialize)]
struct PatchCluster {
    id: String,
    area: String,
    status: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct RegistryPackage {
    workspace: String,
    #[serde(default)]
    package: Option<String>,
    registry: String,
    version: String,
    status: String,
    #[serde(default)]
    provisional: Option<String>,
}

impl RegistryPackage {
    fn package_identity(&self) -> &str {
        self.package.as_deref().unwrap_or(&self.workspace)
    }
}

#[derive(Debug)]
struct Package {
    id: String,
    name: String,
    version: String,
    manifest_path: PathBuf,
    publishable: bool,
    rust_version: Option<String>,
    license: Option<String>,
    license_file: Option<String>,
    description: Option<String>,
    repository: Option<String>,
    readme: Option<String>,
    dependencies: Vec<Dependency>,
}

#[derive(Debug)]
struct Dependency {
    name: String,
    rename: Option<String>,
    req: String,
    kind: Option<String>,
    path: Option<PathBuf>,
    source: Option<String>,
}

fn is_publication_dependency(dependency: &Dependency) -> bool {
    dependency.kind.is_none() || dependency.kind.as_deref() == Some("build")
}

fn immutable_git_source(source: &str) -> bool {
    source
        .split_once('?')
        .and_then(|(_, query)| query.split('#').next())
        .and_then(|query| {
            query
                .split('&')
                .find_map(|parameter| parameter.strip_prefix("rev="))
        })
        .is_some_and(is_full_sha)
}

fn main() {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| usage(1));
    let root = workspace_root();
    let result = match command.as_str() {
        "fork" => match args.next().as_deref() {
            Some("validate") => validate_fork(&root),
            _ => Err("usage: cargo run -p xtask -- fork validate".into()),
        },
        "publish-plan" => publish_plan(&root),
        "release-check" => release_check(&root, args.any(|arg| arg == "--require-registry")),
        _ => Err(format!("unknown command `{command}`")),
    };
    if let Err(error) = result {
        eprintln!("xtask: {error}");
        std::process::exit(1);
    }
}

fn usage(code: i32) -> ! {
    eprintln!(
        "usage: cargo run -p xtask -- <fork validate|publish-plan|release-check [--require-registry]>"
    );
    std::process::exit(code);
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn read_fork(root: &Path) -> Result<ForkMetadata, String> {
    let text = fs::read_to_string(root.join("fork.toml")).map_err(|error| error.to_string())?;
    toml::from_str(&text).map_err(|error| format!("fork.toml: {error}"))
}

fn validate_fork(root: &Path) -> Result<(), String> {
    let metadata = read_fork(root)?;
    validate_fork_metadata(&metadata)?;
    println!(
        "fork metadata valid: {} patch clusters",
        metadata.patch_clusters.len()
    );
    Ok(())
}

fn validate_fork_metadata(metadata: &ForkMetadata) -> Result<(), String> {
    let mut errors = Vec::new();
    if metadata.schema_version == 0 {
        errors.push("schema-version must be positive".to_owned());
    }
    for (field, value) in [
        ("upstream-repository", &metadata.upstream_repository),
        ("fork-repository", &metadata.fork_repository),
        ("upstream-base-commit", &metadata.upstream_base_commit),
        (
            "last-synchronization-date",
            &metadata.last_synchronization_date,
        ),
    ] {
        if value.trim().is_empty() {
            errors.push(format!("{field} must be nonempty"));
        }
    }
    if !metadata.upstream_repository.starts_with("https://") {
        errors.push("upstream-repository must be an https URL".to_owned());
    }
    if !metadata.fork_repository.starts_with("https://") {
        errors.push("fork-repository must be an https URL".to_owned());
    }
    if !is_full_sha(&metadata.upstream_base_commit) {
        errors.push("upstream-base-commit must be a full 40-character SHA-1".to_owned());
    }
    if metadata.last_synchronization_date.len() != 10
        || metadata.last_synchronization_date.as_bytes().get(4) != Some(&b'-')
        || metadata.last_synchronization_date.as_bytes().get(7) != Some(&b'-')
    {
        errors.push("last-synchronization-date must use YYYY-MM-DD".to_owned());
    }
    let mut ids = BTreeSet::new();
    for cluster in &metadata.patch_clusters {
        if cluster.id.trim().is_empty() {
            errors.push("patch cluster id must be nonempty".to_owned());
        } else if !ids.insert(&cluster.id) {
            errors.push(format!("duplicate patch cluster id `{}`", cluster.id));
        }
        if cluster.area.trim().is_empty() {
            errors.push(format!(
                "patch cluster `{}` area must be nonempty",
                cluster.id
            ));
        }
        if cluster.reason.trim().is_empty() {
            errors.push(format!(
                "patch cluster `{}` reason must be nonempty",
                cluster.id
            ));
        }
        if !VALID_STATUSES.contains(&cluster.status.as_str()) {
            errors.push(format!(
                "patch cluster `{}` has invalid status `{}`",
                cluster.id, cluster.status
            ));
        }
    }
    let mut workspace_ids = BTreeSet::new();
    let mut package_ids = BTreeSet::new();
    for package in &metadata.registry_packages {
        if package.workspace.trim().is_empty()
            || package.registry.trim().is_empty()
            || package.version.trim().is_empty()
            || package.status.trim().is_empty()
            || package.package_identity().trim().is_empty()
        {
            errors.push(
                "registry package entries require workspace, registry, version, and status"
                    .to_owned(),
            );
        }
        if !VALID_REGISTRY_STATUSES.contains(&package.status.as_str()) {
            errors.push(format!(
                "registry package `{}` has invalid status `{}`",
                package.workspace, package.status
            ));
        }
        if !workspace_ids.insert(&package.workspace) {
            errors.push(format!(
                "duplicate registry package workspace `{}`",
                package.workspace
            ));
        }
        if !package_ids.insert(package.package_identity()) {
            errors.push(format!(
                "duplicate registry package package identity `{}`",
                package.package_identity()
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn is_full_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn cargo_metadata(root: &Path) -> Result<Vec<Package>, String> {
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    let packages = value
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata did not return packages".to_owned())?;
    packages.iter().map(parse_package).collect()
}

fn parse_package(value: &Value) -> Result<Package, String> {
    let string = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let manifest_path =
        PathBuf::from(string("manifest_path").ok_or("package manifest_path missing")?);
    let publishable = value
        .get("publish")
        .map(|value| value.is_null() || value.as_array().is_some_and(|values| !values.is_empty()))
        .unwrap_or(true);
    let dependencies = value
        .get("dependencies")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "{} dependencies missing",
                string("name").unwrap_or_default()
            )
        })?
        .iter()
        .map(|dependency| {
            Ok(Dependency {
                name: dependency
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                rename: dependency
                    .get("rename")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                req: dependency
                    .get("req")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                kind: dependency
                    .get("kind")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                path: dependency
                    .get("path")
                    .and_then(Value::as_str)
                    .map(PathBuf::from),
                source: dependency
                    .get("source")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Package {
        id: string("id").ok_or("package id missing")?,
        name: string("name").ok_or("package name missing")?,
        version: string("version").ok_or("package version missing")?,
        manifest_path,
        publishable,
        rust_version: string("rust_version"),
        license: string("license"),
        license_file: string("license_file"),
        description: string("description"),
        repository: string("repository"),
        readme: string("readme"),
        dependencies,
    })
}

fn publishable_map(packages: &[Package]) -> BTreeMap<String, usize> {
    packages
        .iter()
        .enumerate()
        .filter(|(_, package)| package.publishable)
        .map(|(index, package)| (package.name.clone(), index))
        .collect()
}

fn dependency_target<'a>(dependency: &Dependency, packages: &'a [Package]) -> Option<&'a Package> {
    let path = dependency.path.as_ref()?;
    packages.iter().find(|package| {
        package.manifest_path.parent().is_some_and(|parent| {
            parent == path || parent == path.canonicalize().ok().as_deref().unwrap_or(path)
        })
    })
}

fn registry_entry_for_package<'a>(
    registry: &'a BTreeMap<String, &'a RegistryPackage>,
    package: &Package,
) -> Option<&'a RegistryPackage> {
    registry.get(&package.name).copied()
}

fn identity_selection_unresolved(status: &str) -> bool {
    matches!(
        status,
        "conflict" | "unavailable" | "owner-decision-required"
    )
}

fn publish_plan(root: &Path) -> Result<(), String> {
    validate_fork(root)?;
    let fork = read_fork(root)?;
    let registry = fork
        .registry_packages
        .iter()
        .map(|package| (package.package_identity().to_owned(), package))
        .collect::<BTreeMap<_, _>>();
    let packages = cargo_metadata(root)?;
    let public = publishable_map(&packages);
    let registry_map_issues = registry_issues(&fork, &packages);
    let root_patches = patched_crates(root)?;
    let reachable_patches = reachable_root_patches(root, &packages, &root_patches)?;
    let mut prerequisites = BTreeMap::<String, BTreeSet<String>>::new();
    let mut blocked = BTreeMap::<String, BTreeSet<String>>::new();
    for package in packages.iter().filter(|package| package.publishable) {
        let entry = prerequisites.entry(package.name.clone()).or_default();
        for dependency in package
            .dependencies
            .iter()
            .filter(|dependency| is_publication_dependency(dependency))
        {
            if let Some(target) = dependency_target(dependency, &packages) {
                if public.contains_key(&target.name) {
                    entry.insert(target.name.clone());
                } else {
                    blocked
                        .entry(package.name.clone())
                        .or_default()
                        .insert(target.name.clone());
                }
            }
        }
    }
    let order = topological_order(&prerequisites)?;
    println!("# GPUI engine publication plan (local graph; no publication performed)");
    for (position, name) in order.iter().enumerate() {
        let package = &packages[*public.get(name).expect("plan package")];
        let prereqs = prerequisites[name]
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let blocked = blocked.get(name).cloned().unwrap_or_default();
        let blocked_names = blocked.iter().cloned().collect::<Vec<_>>().join(", ");
        let patches = reachable_patches
            .get(name)
            .expect("reachable patch set for publishable package");
        let patch_names = patches.iter().cloned().collect::<Vec<_>>().join(", ");
        let registry_blocked = prerequisites[name].iter().any(|prerequisite| {
            public
                .get(prerequisite)
                .and_then(|index| registry_entry_for_package(&registry, &packages[*index]))
                .is_none_or(|entry| entry.status != "published")
        });
        let registry_entry = registry_entry_for_package(&registry, package);
        println!(
            "{}. {} -> {}@{} (repository: {}; version {}; published: {}; prerequisites: {}; metadata: {}; root patches: {}; full-dry-run: {}; blocked: {})",
            position + 1,
            package.name,
            registry_entry
                .map(|entry| entry.registry.as_str())
                .unwrap_or("unmapped"),
            package.version,
            package.repository.as_deref().unwrap_or("unmapped"),
            package.version,
            registry_entry
                .map(|entry| entry.status.as_str())
                .unwrap_or("unmapped"),
            if prereqs.is_empty() { "none" } else { &prereqs },
            metadata_ready(package),
            if patch_names.is_empty() {
                "none"
            } else {
                &patch_names
            },
            if full_dry_run_possible(patches, registry_blocked, &blocked) {
                "possible"
            } else {
                "blocked"
            },
            if blocked_names.is_empty() {
                "none"
            } else {
                &blocked_names
            }
        );
    }
    for issue in registry_map_issues {
        println!("registry-map: {issue}");
    }
    Ok(())
}

fn full_dry_run_possible(
    root_patches: &BTreeSet<String>,
    registry_blocked: bool,
    blocked: &BTreeSet<String>,
) -> bool {
    root_patches.is_empty() && !registry_blocked && blocked.is_empty()
}

fn metadata_ready(package: &Package) -> &'static str {
    if package.rust_version.is_some()
        && (package
            .license
            .as_ref()
            .is_some_and(|value| !value.is_empty())
            || package
                .license_file
                .as_ref()
                .is_some_and(|value| !value.is_empty()))
        && package
            .description
            .as_ref()
            .is_some_and(|value| !value.is_empty())
        && package
            .repository
            .as_ref()
            .is_some_and(|value| !value.is_empty())
        && package
            .readme
            .as_ref()
            .is_some_and(|value| !value.is_empty())
    {
        "ready"
    } else {
        "incomplete"
    }
}

fn topological_order(
    prerequisites: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<String>, String> {
    let mut indegree: BTreeMap<String, usize> = prerequisites
        .keys()
        .map(|name| (name.clone(), prerequisites[name].len()))
        .collect();
    let mut dependents = BTreeMap::<String, BTreeSet<String>>::new();
    for (package, dependencies) in prerequisites {
        for dependency in dependencies {
            dependents
                .entry(dependency.clone())
                .or_default()
                .insert(package.clone());
        }
    }
    let mut ready = VecDeque::from_iter(
        indegree
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(name, _)| name.clone()),
    );
    let mut order = Vec::new();
    while let Some(name) = ready.pop_front() {
        order.push(name.clone());
        if let Some(children) = dependents.get(&name) {
            for child in children {
                let count = indegree.get_mut(child).expect("dependent in graph");
                *count -= 1;
                if *count == 0 {
                    ready.push_back(child.clone());
                }
            }
        }
    }
    if order.len() != indegree.len() {
        Err("publish graph contains a cycle".to_owned())
    } else {
        Ok(order)
    }
}

fn release_check(root: &Path, require_registry: bool) -> Result<(), String> {
    validate_fork(root)?;
    let metadata = read_fork(root)?;
    let packages = cargo_metadata(root)?;
    let public = publishable_map(&packages);
    let mut manifest_errors = Vec::new();
    let mut registry_errors = registry_issues(&metadata, &packages);
    manifest_errors.extend(source_gate_issues(root));
    manifest_errors.extend(package_artifact_issues(root, &packages));
    manifest_errors.extend(package_dry_run_issues(root, &packages));
    let root_patches = patched_crates(root)?;
    let reachable_patches = reachable_root_patches(root, &packages, &root_patches)?;
    for package in packages.iter().filter(|package| package.publishable) {
        if metadata_ready(package) != "ready" {
            manifest_errors.push(format!(
                "{} is missing rust-version, license, description, repository, or readme metadata",
                package.name
            ));
        }
        for patch in reachable_patches.get(&package.name).into_iter().flatten() {
            manifest_errors.push(format!(
                "{} depends on `{patch}` through root [patch.crates-io]; packaged manifests drop root patches",
                package.name
            ));
        }
        for dependency in package
            .dependencies
            .iter()
            .filter(|dependency| is_publication_dependency(dependency))
        {
            manifest_errors.extend(dependency_errors(package, dependency, &packages, &public));
        }
    }
    let identity_blockers: Vec<_> = metadata
        .registry_packages
        .iter()
        .filter(|package| identity_selection_unresolved(&package.status))
        .map(|package| {
            format!(
                "{} -> {}@{} status {}; provisional name {}",
                package.workspace,
                package.registry,
                package.version,
                package.status,
                package.provisional.as_deref().unwrap_or("not recorded")
            )
        })
        .collect();
    if !identity_blockers.is_empty() {
        registry_errors.push(format!(
            "identity selection blocked: {}",
            identity_blockers.join(" | ")
        ));
    }
    if require_registry {
        for package in &metadata.registry_packages {
            if package.status != "published" {
                registry_errors.push(format!(
                    "--require-registry: {} -> {}@{} is not published (status {})",
                    package.workspace, package.registry, package.version, package.status
                ));
            }
        }
    }
    let mut errors = manifest_errors
        .into_iter()
        .map(|error| format!("manifest: {error}"))
        .collect::<Vec<_>>();
    errors.extend(
        registry_errors
            .into_iter()
            .map(|error| format!("registry: {error}")),
    );
    if errors.is_empty() {
        println!("release check passed (source/package graph only; no publication performed)");
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn source_gate_issues(root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    for (name, args) in SOURCE_GATES {
        let output = Command::new("cargo").args(*args).current_dir(root).output();
        let Ok(output) = output else {
            errors.push(format!("source gate {name} could not start"));
            continue;
        };
        if !output.status.success() {
            errors.push(format!(
                "source gate {name} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
            continue;
        }
        println!("source gate passed: {name}");
    }
    errors
}

fn package_artifact_issues(root: &Path, packages: &[Package]) -> Vec<String> {
    let mut errors = Vec::new();
    for package in packages.iter().filter(|package| package.publishable) {
        let output = Command::new("cargo")
            .args([
                "--locked",
                "package",
                "--allow-dirty",
                "--no-verify",
                "--list",
                "-p",
                &package.name,
            ])
            .current_dir(root)
            .output();
        let Ok(output) = output else {
            errors.push(format!(
                "{} package file-list command could not start",
                package.name
            ));
            continue;
        };
        if !output.status.success() {
            errors.push(format!(
                "{} package file-list failed: {}",
                package.name,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
            continue;
        }
        let files = String::from_utf8_lossy(&output.stdout);
        if !files.lines().any(|line| line == "README.md") {
            errors.push(format!("{} package artifact omits README.md", package.name));
        }
        let package_dir = package.manifest_path.parent().unwrap_or(Path::new("."));
        let license_files = fs::read_dir(package_dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("LICENSE")
            })
            .collect::<Vec<_>>();
        if license_files.is_empty() {
            errors.push(format!(
                "{} has no license text file; manifest license alone is not artifact evidence",
                package.name
            ));
        }
        for entry in license_files {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !files.lines().any(|line| line == name) {
                errors.push(format!(
                    "{} package artifact omits license text {}",
                    package.name, name
                ));
            }
            if entry.path().is_symlink() && fs::canonicalize(entry.path()).is_err() {
                errors.push(format!(
                    "{} license link {} cannot be read",
                    package.name,
                    entry.path().display()
                ));
            }
        }
    }
    errors
}

fn package_dry_run_issues(root: &Path, packages: &[Package]) -> Vec<String> {
    let mut errors = Vec::new();
    for package in packages.iter().filter(|package| package.publishable) {
        let output = Command::new("cargo")
            .args([
                "--locked",
                "package",
                "--allow-dirty",
                "--no-verify",
                "-p",
                &package.name,
            ])
            .current_dir(root)
            .output();
        let Ok(output) = output else {
            errors.push(format!(
                "{} normalized manifest check could not start",
                package.name
            ));
            continue;
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let class = if stderr.contains("no matching package named")
                || stderr.contains("failed to select a version")
                || stderr.contains("could not find a matching version")
            {
                "registry prerequisite"
            } else {
                "package validation"
            };
            errors.push(format!(
                "{} {}/normalized manifest check blocked: {}",
                package.name,
                class,
                stderr.trim()
            ));
        } else {
            errors.extend(normalized_archive_issues(root, package, packages));
        }
    }
    errors
}

fn normalized_archive_issues(root: &Path, package: &Package, packages: &[Package]) -> Vec<String> {
    let archive = root
        .join("target/package")
        .join(format!("{}-{}.crate", package.name, package.version));
    let member = format!("{}-{}/Cargo.toml", package.name, package.version);
    let output = Command::new("tar")
        .args(["-xOf", archive.to_string_lossy().as_ref(), &member])
        .output();
    let Ok(output) = output else {
        return vec![format!(
            "{} normalized manifest unavailable: tar command could not start",
            package.name
        )];
    };
    if !output.status.success() {
        return vec![format!(
            "{} normalized manifest unavailable: {}",
            package.name,
            String::from_utf8_lossy(&output.stderr).trim()
        )];
    }
    let normalized = String::from_utf8_lossy(&output.stdout);
    normalized_manifest_issues(package, packages, &normalized)
}

fn normalized_manifest_issues(
    package: &Package,
    packages: &[Package],
    normalized: &str,
) -> Vec<String> {
    let normalized = match toml::from_str::<toml::Value>(normalized) {
        Ok(normalized) => normalized,
        Err(error) => {
            return vec![format!(
                "{} normalized manifest is invalid: {error}",
                package.name
            )];
        }
    };
    let dependency_tables = normal_build_dependency_tables(&normalized);
    let mut errors = Vec::new();
    for dependency in dependency_tables
        .iter()
        .flat_map(|table| table.values())
        .filter_map(toml::Value::as_table)
    {
        if dependency.contains_key("git") {
            errors.push(format!(
                "{} normalized manifest retains a Git dependency",
                package.name
            ));
        }
        if dependency.contains_key("path") {
            errors.push(format!(
                "{} normalized manifest retains a path dependency",
                package.name
            ));
        }
    }
    for dependency in package
        .dependencies
        .iter()
        .filter(|dependency| is_publication_dependency(dependency))
    {
        let Some(target) = dependency_target(dependency, packages) else {
            continue;
        };
        let alias = dependency.rename.as_deref().unwrap_or(&dependency.name);
        let Some(normalized_dependency) =
            dependency_tables.iter().find_map(|table| table.get(alias))
        else {
            errors.push(format!(
                "{} normalized manifest lost dependency alias {}",
                package.name, alias
            ));
            continue;
        };
        if dependency.rename.is_some()
            && normalized_dependency
                .as_table()
                .and_then(|dependency| dependency.get("package"))
                .and_then(toml::Value::as_str)
                != Some(&target.name)
        {
            errors.push(format!(
                "{} normalized manifest changed dependency package {} for alias {}",
                package.name, target.name, alias
            ));
        }
        let exact_version = format!("={}", target.version);
        if normalized_dependency_version(normalized_dependency) != Some(&exact_version) {
            errors.push(format!(
                "{} normalized manifest lost exact sibling version {} for {}",
                package.name, target.version, alias
            ));
        }
    }
    if normalized
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("readme"))
        .and_then(toml::Value::as_str)
        != Some("README.md")
    {
        errors.push(format!(
            "{} normalized manifest does not retain README.md",
            package.name
        ));
    }
    errors
}

fn normal_build_dependency_tables(manifest: &toml::Value) -> Vec<&toml::Table> {
    let mut tables = ["dependencies", "build-dependencies"]
        .into_iter()
        .filter_map(|name| manifest.get(name).and_then(toml::Value::as_table))
        .collect::<Vec<_>>();
    let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) else {
        return tables;
    };
    tables.extend(
        targets
            .values()
            .filter_map(toml::Value::as_table)
            .flat_map(|target| {
                ["dependencies", "build-dependencies"]
                    .into_iter()
                    .filter_map(move |name| target.get(name).and_then(toml::Value::as_table))
            }),
    );
    tables
}

fn normalized_dependency_version(dependency: &toml::Value) -> Option<&str> {
    dependency.as_str().or_else(|| {
        dependency
            .as_table()
            .and_then(|dependency| dependency.get("version"))
            .and_then(toml::Value::as_str)
    })
}

fn registry_issues(metadata: &ForkMetadata, packages: &[Package]) -> Vec<String> {
    let entries = metadata
        .registry_packages
        .iter()
        .map(|entry| (entry.package_identity().to_owned(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut issues = Vec::new();
    for package in packages.iter().filter(|package| package.publishable) {
        match registry_entry_for_package(&entries, package) {
            None => issues.push(format!("missing registry map entry for {}", package.name)),
            Some(entry) if entry.version != package.version => issues.push(format!(
                "{} registry map version {} does not match package {}",
                package.name, entry.version, package.version
            )),
            Some(_) => {}
        }
    }
    for entry in &metadata.registry_packages {
        if !packages
            .iter()
            .any(|package| package.publishable && package.name == entry.package_identity())
        {
            issues.push(format!(
                "registry map entry {} does not name a publishable package {}",
                entry.workspace,
                entry.package_identity()
            ));
        }
    }
    issues
}

fn dependency_errors(
    package: &Package,
    dependency: &Dependency,
    packages: &[Package],
    public: &BTreeMap<String, usize>,
) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(target) = dependency_target(dependency, packages) {
        if !public.contains_key(&target.name) {
            errors.push(format!(
                "{} has private normal dependency {}",
                package.name, target.name
            ));
        } else if dependency.req != format!("={}", target.version) {
            errors.push(format!(
                "{} -> {} requires `{}`, expected `={}`",
                package.name, target.name, dependency.req, target.version
            ));
        }
    } else if let Some(source) = &dependency.source
        && source.starts_with("git+")
    {
        if !immutable_git_source(source) {
            errors.push(format!(
                "{} has mutable Git dependency {} (requires ?rev=<full-sha>)",
                package.name, dependency.name
            ));
        }
        if !dependency.req.starts_with('=') {
            errors.push(format!(
                "{} has non-exact Git dependency {} (requires =<version>)",
                package.name, dependency.name
            ));
        }
    }
    errors
}

fn patched_crates(root: &Path) -> Result<BTreeSet<String>, String> {
    let text = fs::read_to_string(root.join("Cargo.toml")).map_err(|error| error.to_string())?;
    let mut names = BTreeSet::new();
    let mut in_patch_table = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_patch_table = trimmed == "[patch.crates-io]";
            continue;
        }
        if in_patch_table
            && !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && let Some(name) = trimmed.split('=').next().map(str::trim)
            && !name.is_empty()
        {
            names.insert(name.trim_matches('"').replace('_', "-"));
        }
    }
    Ok(names)
}

fn reachable_root_patches(
    root: &Path,
    packages: &[Package],
    root_patches: &BTreeSet<String>,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cargo metadata: {error}"))?;
    let public = packages
        .iter()
        .filter(|package| package.publishable)
        .map(|package| (package.name.clone(), package.id.clone()))
        .collect();
    reachable_root_patches_from_metadata(&metadata, &public, root_patches)
}

fn reachable_root_patches_from_metadata(
    metadata: &Value,
    public: &BTreeMap<String, String>,
    root_patches: &BTreeSet<String>,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata did not return packages".to_owned())?;
    let names_by_id = packages
        .iter()
        .filter_map(|package| {
            Some((
                package.get("id")?.as_str()?.to_owned(),
                package.get("name")?.as_str()?.to_owned(),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    if !public.values().all(|id| names_by_id.contains_key(id)) {
        return Err("cargo metadata did not return every publishable package".to_owned());
    }
    let nodes = metadata
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata did not return a resolved dependency graph".to_owned())?;
    let dependencies = nodes
        .iter()
        .filter_map(|node| {
            let id = node.get("id")?.as_str()?.to_owned();
            let dependencies = node
                .get("deps")?
                .as_array()?
                .iter()
                .filter(|dependency| {
                    dependency
                        .get("dep_kinds")
                        .and_then(Value::as_array)
                        .is_some_and(|kinds| {
                            kinds
                                .iter()
                                .any(|kind| kind.get("kind").and_then(Value::as_str) != Some("dev"))
                        })
                })
                .filter_map(|dependency| dependency.get("pkg").and_then(Value::as_str))
                .map(str::to_owned)
                .collect();
            Some((id, dependencies))
        })
        .collect::<BTreeMap<_, Vec<_>>>();
    let mut reachable = BTreeMap::new();
    for (package, id) in public {
        let mut queue = VecDeque::from([id.clone()]);
        let mut visited = BTreeSet::new();
        let mut patches = BTreeSet::new();
        while let Some(id) = queue.pop_front() {
            if !visited.insert(id.clone()) {
                continue;
            }
            if names_by_id
                .get(&id)
                .is_some_and(|name| root_patches.contains(name))
            {
                patches.insert(names_by_id[&id].clone());
            }
            queue.extend(dependencies.get(&id).into_iter().flatten().cloned());
        }
        reachable.insert(package.clone(), patches);
    }
    Ok(reachable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_sha_validation_rejects_short_and_non_hex_values() {
        assert!(!is_full_sha("abc"));
        assert!(!is_full_sha(&"g".repeat(40)));
        assert!(is_full_sha(&"a".repeat(40)));
    }

    #[test]
    fn git_source_with_full_revision_query_is_immutable() {
        let revision = "a".repeat(40);
        assert!(immutable_git_source(&format!(
            "git+https://example.invalid/repository?rev={revision}"
        )));
        assert!(!immutable_git_source(
            "git+https://example.invalid/repository?rev=0123456789abcdef"
        ));
    }

    #[test]
    fn normalized_manifest_ignores_paths_outside_dependency_tables() {
        let packages = vec![test_package("consumer", "1.0.0", true, "/tmp/consumer")];
        let normalized = r#"
            [package]
            readme = "README.md"

            [lib]
            path = "src/lib.rs"
        "#;

        assert!(normalized_manifest_issues(&packages[0], &packages, normalized).is_empty());
    }

    #[test]
    fn normalized_manifest_retains_target_specific_dependency_aliases() {
        let mut consumer = test_package("consumer", "1.0.0", true, "/tmp/consumer");
        consumer.dependencies.push(Dependency {
            name: "gpui-util".into(),
            rename: Some("util".into()),
            req: "=1.0.0".into(),
            kind: None,
            path: Some("/tmp/gpui-util".into()),
            source: None,
        });
        let sibling = test_package("gpui-util", "1.0.0", true, "/tmp/gpui-util");
        let packages = vec![consumer, sibling];
        let normalized = r#"
            [package]
            readme = "README.md"

            [target.'cfg(not(target_family = "wasm"))'.dependencies.util]
            version = "=1.0.0"
            package = "gpui-util"
        "#;

        assert!(normalized_manifest_issues(&packages[0], &packages, normalized).is_empty());
    }

    #[test]
    fn source_gates_match_locked_ci_checks() {
        assert_eq!(
            SOURCE_GATES,
            [
                (
                    "workspace all-target check",
                    &["check", "--locked", "--workspace", "--all-targets"][..],
                ),
                (
                    "scheduler tests",
                    &["test", "--locked", "-p", "scheduler"][..],
                ),
                (
                    "gpui library tests",
                    &[
                        "test",
                        "--locked",
                        "-p",
                        "bumpyclock-gpui",
                        "--features",
                        "bench,profiler",
                        "--lib",
                    ][..],
                ),
                (
                    "gpui wgpu library tests",
                    &["test", "--locked", "-p", "gpui_wgpu", "--lib"][..],
                ),
                (
                    "gpui examples",
                    &["check", "--locked", "-p", "bumpyclock-gpui", "--examples"][..],
                ),
            ]
        );
    }

    #[test]
    fn invalid_status_is_not_in_ledger_vocabulary() {
        assert!(!VALID_STATUSES.contains(&"maybe"));
        assert!(VALID_STATUSES.contains(&"fork-only"));
        assert!(VALID_REGISTRY_STATUSES.contains(&"selected-unpublished"));
    }

    #[test]
    fn metadata_requires_all_publication_fields() {
        let package = Package {
            id: "test".into(),
            name: "test".into(),
            version: "0.1.0".into(),
            manifest_path: "Cargo.toml".into(),
            publishable: true,
            rust_version: None,
            license: Some("Apache-2.0".into()),
            license_file: None,
            description: Some("desc".into()),
            repository: Some("https://example.invalid".into()),
            readme: None,
            dependencies: Vec::new(),
        };
        assert_eq!(metadata_ready(&package), "incomplete");
    }

    #[test]
    fn fork_validation_rejects_duplicate_ids_missing_reasons_and_bad_statuses() {
        let metadata = ForkMetadata {
            schema_version: 1,
            upstream_repository: "https://github.com/zed-industries/zed".into(),
            upstream_base_commit: "a".repeat(40),
            last_synchronization_date: "2026-07-09".into(),
            fork_repository: "https://github.com/BumpyClock/gpui".into(),
            patch_clusters: vec![
                PatchCluster {
                    id: "same".into(),
                    area: "area".into(),
                    status: "fork-only".into(),
                    reason: "reason".into(),
                },
                PatchCluster {
                    id: "same".into(),
                    area: "area".into(),
                    status: "invalid".into(),
                    reason: String::new(),
                },
            ],
            registry_packages: Vec::new(),
        };
        let error = validate_fork_metadata(&metadata).expect_err("invalid ledger should fail");
        assert!(error.contains("duplicate patch cluster id"));
        assert!(error.contains("invalid status"));
        assert!(error.contains("reason must be nonempty"));
    }

    fn test_package(name: &str, version: &str, publishable: bool, path: &str) -> Package {
        Package {
            id: name.into(),
            name: name.into(),
            version: version.into(),
            manifest_path: PathBuf::from(path).join("Cargo.toml"),
            publishable,
            rust_version: Some("1.95.0".into()),
            license: Some("Apache-2.0".into()),
            license_file: Some("LICENSE-APACHE".into()),
            description: Some("test".into()),
            repository: Some("https://example.invalid".into()),
            readme: Some("README.md".into()),
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn dependency_checks_reject_private_git_only_and_non_exact_requirements() {
        let consumer = test_package("consumer", "1.0.0", true, "/tmp/consumer");
        let private = test_package("private", "1.0.0", false, "/tmp/private");
        let public = test_package("public", "1.0.0", true, "/tmp/public");
        let packages = vec![consumer, private, public];
        let map = publishable_map(&packages);
        let private_dependency = Dependency {
            name: "private".into(),
            rename: None,
            req: "=1.0.0".into(),
            kind: None,
            path: Some("/tmp/private".into()),
            source: None,
        };
        assert!(
            dependency_errors(&packages[0], &private_dependency, &packages, &map,)
                .iter()
                .any(|error| error.contains("private normal dependency"))
        );

        let mismatched = Dependency {
            name: "public".into(),
            rename: None,
            req: "^1.0.0".into(),
            kind: None,
            path: Some("/tmp/public".into()),
            source: None,
        };
        assert!(
            dependency_errors(&packages[0], &mismatched, &packages, &map)
                .iter()
                .any(|error| error.contains("expected `=1.0.0`"))
        );

        let git_only = Dependency {
            name: "git-only".into(),
            rename: None,
            req: "*".into(),
            kind: None,
            path: None,
            source: Some("git+https://example.invalid/repo".into()),
        };
        assert!(
            dependency_errors(&packages[0], &git_only, &packages, &map)
                .iter()
                .any(|error| error.contains("non-exact Git"))
        );
        let mutable_git = Dependency {
            name: "mutable-git".into(),
            rename: None,
            req: "=1.0.0".into(),
            kind: None,
            path: None,
            source: Some("git+https://example.invalid/repo".into()),
        };
        assert!(
            dependency_errors(&packages[0], &mutable_git, &packages, &map,)
                .iter()
                .any(|error| error.contains("mutable Git"))
        );
    }

    #[test]
    fn build_dependencies_are_publication_prerequisites_but_dev_dependencies_are_not() {
        let build = Dependency {
            name: "build".into(),
            rename: None,
            req: "*".into(),
            kind: Some("build".into()),
            path: None,
            source: None,
        };
        let dev = Dependency {
            name: "dev".into(),
            rename: None,
            req: "*".into(),
            kind: Some("dev".into()),
            path: None,
            source: None,
        };
        assert!(is_publication_dependency(&build));
        assert!(!is_publication_dependency(&dev));
    }

    #[test]
    fn registry_status_vocabulary_rejects_untracked_states() {
        let metadata = ForkMetadata {
            schema_version: 1,
            upstream_repository: "https://github.com/zed-industries/zed".into(),
            upstream_base_commit: "a".repeat(40),
            last_synchronization_date: "2026-07-09".into(),
            fork_repository: "https://github.com/BumpyClock/gpui".into(),
            patch_clusters: Vec::new(),
            registry_packages: vec![RegistryPackage {
                workspace: "gpui".into(),
                package: None,
                registry: "gpui".into(),
                version: "0.2.2".into(),
                status: "maybe".into(),
                provisional: None,
            }],
        };
        let error = validate_fork_metadata(&metadata).expect_err("unknown status should fail");
        assert!(error.contains("invalid status"));
    }

    #[test]
    fn selected_unpublished_is_not_an_identity_collision() {
        assert!(!identity_selection_unresolved("selected-unpublished"));
        assert!(identity_selection_unresolved("conflict"));
    }

    #[test]
    fn registry_map_uses_explicit_package_identity() {
        let package = test_package("bumpyclock-gpui", "0.1.0", true, "/tmp/gpui");
        let entry = RegistryPackage {
            workspace: "gpui".into(),
            package: Some("bumpyclock-gpui".into()),
            registry: "bumpyclock-gpui".into(),
            version: "0.1.0".into(),
            status: "selected-unpublished".into(),
            provisional: None,
        };
        let registry = BTreeMap::from([(entry.package_identity().to_owned(), &entry)]);

        assert_eq!(
            registry_entry_for_package(&registry, &package)
                .expect("explicit package identity")
                .workspace,
            "gpui"
        );
    }

    #[test]
    fn fork_validation_rejects_duplicate_package_identities() {
        let metadata = ForkMetadata {
            schema_version: 1,
            upstream_repository: "https://github.com/zed-industries/zed".into(),
            upstream_base_commit: "a".repeat(40),
            last_synchronization_date: "2026-07-09".into(),
            fork_repository: "https://github.com/BumpyClock/gpui".into(),
            patch_clusters: Vec::new(),
            registry_packages: vec![
                RegistryPackage {
                    workspace: "first".into(),
                    package: Some("shared-package".into()),
                    registry: "shared-package".into(),
                    version: "0.1.0".into(),
                    status: "unavailable".into(),
                    provisional: None,
                },
                RegistryPackage {
                    workspace: "second".into(),
                    package: Some("shared-package".into()),
                    registry: "shared-package".into(),
                    version: "0.1.0".into(),
                    status: "unavailable".into(),
                    provisional: None,
                },
            ],
        };

        let error = validate_fork_metadata(&metadata).expect_err("duplicate package identity");
        assert!(error.contains("duplicate registry package package identity"));
    }

    #[test]
    fn root_patch_audit_finds_source_only_overrides() {
        let patches = patched_crates(&workspace_root()).expect("root Cargo.toml");
        assert!(patches.contains("async-task"));
        assert!(patches.contains("calloop"));
        assert!(patches.contains("windows-capture"));
    }

    #[test]
    fn root_patch_audit_tracks_transitive_non_dev_dependencies() {
        let metadata = serde_json::json!({
            "packages": [
                { "id": "consumer", "name": "consumer" },
                { "id": "scap", "name": "zed-scap" },
                { "id": "windows-capture", "name": "windows-capture" },
                { "id": "dev-only", "name": "dev-only" },
            ],
            "resolve": {
                "nodes": [
                    {
                        "id": "consumer",
                        "deps": [
                            { "pkg": "scap", "dep_kinds": [{ "kind": null }] },
                            { "pkg": "dev-only", "dep_kinds": [{ "kind": "dev" }] },
                        ],
                    },
                    {
                        "id": "scap",
                        "deps": [{
                            "pkg": "windows-capture",
                            "dep_kinds": [{ "kind": null, "target": "cfg(target_os = \"windows\")" }],
                        }],
                    },
                    { "id": "windows-capture", "deps": [] },
                    { "id": "dev-only", "deps": [] },
                ],
            },
        });
        let reachable = reachable_root_patches_from_metadata(
            &metadata,
            &BTreeMap::from([("consumer".to_owned(), "consumer".to_owned())]),
            &BTreeSet::from(["windows-capture".to_owned(), "dev-only".to_owned()]),
        )
        .expect("valid metadata");

        assert_eq!(
            reachable["consumer"],
            BTreeSet::from(["windows-capture".to_owned()])
        );
    }

    #[test]
    fn publication_plan_blocks_packages_with_reachable_root_patches() {
        assert!(!full_dry_run_possible(
            &BTreeSet::from(["windows-capture".to_owned()]),
            false,
            &BTreeSet::new(),
        ));
        assert!(full_dry_run_possible(
            &BTreeSet::new(),
            false,
            &BTreeSet::new(),
        ));
    }

    #[test]
    fn publication_plan_rejects_cycles() {
        let mut graph = BTreeMap::new();
        graph.insert("a".into(), BTreeSet::from(["b".into()]));
        graph.insert("b".into(), BTreeSet::from(["a".into()]));
        assert!(
            topological_order(&graph)
                .expect_err("cycle should be rejected")
                .contains("cycle")
        );
    }
}
