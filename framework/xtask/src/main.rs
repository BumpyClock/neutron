use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use toml::Value;
use wait_timeout::ChildExt;

const COMPATIBILITY_FILE: &str = "compatibility.toml";
const GENERATED_FILE: &str = "docs/COMPATIBILITY.md";
const VALID_REGISTRY_STATUSES: &[&str] = &[
    "published",
    "unpublished",
    "selected-unpublished",
    "unavailable",
    "conflict",
];
const VALID_EVIDENCE_STATUSES: &[&str] = &["verified", "not-verified", "not-applicable", "blocked"];
const VALID_MATURITY: &[&str] = &["supported", "preview", "experimental"];
const REGISTRY_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Deserialize)]
struct Compatibility {
    schema: u32,
    framework: Framework,
    gpui: Gpui,
    release: Release,
    platforms: Vec<Platform>,
}

#[derive(Debug, Deserialize)]
struct Framework {
    name: String,
    version: String,
    repository: String,
    rust_msrv: String,
    pinned_toolchain: String,
    audit_toolchain: String,
    previous_release: String,
    previous_release_gpui_rev: String,
}

#[derive(Debug, Deserialize)]
struct Gpui {
    repository: String,
    rev: String,
    zed_repository: String,
    zed_upstream_base: String,
    packages: Vec<GpuiPackage>,
}

#[derive(Debug, Deserialize)]
struct GpuiPackage {
    dependency: String,
    registry_package: String,
    version: String,
    crate_path: String,
    public_api: bool,
    features: Vec<String>,
    registry_status: String,
    registry_note: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    crates_io_ready: bool,
    blockers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Platform {
    name: String,
    target: String,
    build: String,
    unit: String,
    headless: String,
    native_runtime: String,
    renderer: String,
    package: String,
    maturity: String,
    notes: String,
}

#[derive(Default)]
struct Options {
    gpui_path: Option<PathBuf>,
    require_registry: bool,
}

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask must be inside the workspace")?;
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_help();
        bail!("missing command");
    };

    match command.as_str() {
        "compatibility" => {
            let Some(action) = args.next() else {
                bail!("usage: cargo xtask compatibility <generate|check> [--gpui-path PATH]");
            };
            let options = parse_options(args)?;
            match action.as_str() {
                "generate" => generate(root, options.gpui_path.as_deref()),
                "check" => check(root, options.gpui_path.as_deref()),
                _ => bail!("unknown compatibility action: {action}"),
            }
        }
        "publish-plan" => {
            let options = parse_options(args)?;
            publish_plan(root, options.gpui_path.as_deref(), options.require_registry)
        }
        "release-check" => {
            let options = parse_options(args)?;
            release_check(root, &options)
        }
        "--help" | "-h" | "help" => {
            print_help();
            Ok(())
        }
        _ => bail!("unknown command: {command}"),
    }
}

fn print_help() {
    println!(
        "\
Usage:
  cargo xtask compatibility generate [--gpui-path PATH]
  cargo xtask compatibility check [--gpui-path PATH]
  cargo xtask publish-plan [--gpui-path PATH] [--require-registry]
  cargo xtask release-check [--gpui-path PATH] [--require-registry]"
    );
}

fn parse_options(mut args: impl Iterator<Item = String>) -> Result<Options> {
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gpui-path" => {
                options.gpui_path = Some(
                    args.next()
                        .map(PathBuf::from)
                        .context("--gpui-path requires a path")?,
                );
            }
            "--require-registry" => options.require_registry = true,
            _ => bail!("unknown option: {arg}"),
        }
    }
    Ok(options)
}

fn load(root: &Path) -> Result<Compatibility> {
    let path = root.join(COMPATIBILITY_FILE);
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&source).with_context(|| format!("invalid {}", path.display()))
}

fn generate(root: &Path, gpui_path: Option<&Path>) -> Result<()> {
    let compatibility = load(root)?;
    let report = validate(root, &compatibility, gpui_path, false);
    print_warnings(&report.warnings);
    if !report.errors.is_empty() {
        return Err(validation_error(report.errors));
    }
    let output = render(&compatibility);
    fs::write(root.join(GENERATED_FILE), output)
        .with_context(|| format!("failed to write {GENERATED_FILE}"))?;
    println!("generated {GENERATED_FILE}");
    Ok(())
}

fn check(root: &Path, gpui_path: Option<&Path>) -> Result<()> {
    let compatibility = load(root)?;
    let mut report = validate(root, &compatibility, gpui_path, true);
    validate_generated(root, &compatibility, &mut report.errors);
    print_warnings(&report.warnings);
    if !report.errors.is_empty() {
        return Err(validation_error(report.errors));
    }
    println!(
        "compatibility valid: {} {} -> {}#{}",
        compatibility.framework.name,
        compatibility.framework.version,
        compatibility.gpui.repository,
        compatibility.gpui.rev
    );
    Ok(())
}

fn validate_generated(root: &Path, compatibility: &Compatibility, errors: &mut Vec<String>) {
    match fs::read_to_string(root.join(GENERATED_FILE)) {
        Ok(actual) if actual == render(compatibility) => {}
        Ok(_) => errors.push(format!(
            "{GENERATED_FILE} is stale; run `cargo xtask compatibility generate`"
        )),
        Err(error) => errors.push(format!("failed to read {GENERATED_FILE}: {error}")),
    }
}

#[derive(Default)]
struct Validation {
    errors: Vec<String>,
    warnings: Vec<String>,
}

fn validation_error(errors: Vec<String>) -> anyhow::Error {
    anyhow!(
        "compatibility validation failed:\n{}",
        errors
            .into_iter()
            .map(|error| format!("- {error}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
}

fn validate(
    root: &Path,
    compatibility: &Compatibility,
    gpui_path: Option<&Path>,
    check_generated: bool,
) -> Validation {
    let mut report = Validation::default();
    validate_metadata(compatibility, &mut report.errors);

    let root_manifest_path = root.join("Cargo.toml");
    let root_manifest = match read_toml(&root_manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            report.errors.push(error.to_string());
            return report;
        }
    };
    validate_root_manifest(&root_manifest, compatibility, &mut report.errors);
    validate_all_manifests(root, compatibility, &root_manifest_path, &mut report.errors);
    validate_lockfile(root, compatibility, &mut report.errors);
    validate_toolchain(root, compatibility, &mut report.errors);
    validate_publishable_dependencies(root, &mut report.errors);

    if let Some(gpui_path) = gpui_path {
        validate_gpui_checkout(gpui_path, compatibility, &mut report);
    }

    if check_generated && !root.join(GENERATED_FILE).is_file() {
        report.errors.push(format!(
            "{GENERATED_FILE} is missing; run `cargo xtask compatibility generate`"
        ));
    }
    report
}

fn validate_metadata(compatibility: &Compatibility, errors: &mut Vec<String>) {
    if compatibility.schema != 1 {
        errors.push(format!(
            "unsupported compatibility schema {}; expected 1",
            compatibility.schema
        ));
    }
    if compatibility.framework.name.trim().is_empty() {
        errors.push("framework.name must not be empty".into());
    }
    if compatibility.framework.repository.trim().is_empty() {
        errors.push("framework.repository must not be empty".into());
    }
    if !is_version(&compatibility.framework.version) {
        errors.push("framework.version must be a three-part numeric version".into());
    }
    if !is_version(&compatibility.framework.previous_release) {
        errors.push("framework.previous_release must be a three-part numeric version".into());
    }
    if !is_msrv(&compatibility.framework.rust_msrv) {
        errors.push("framework.rust_msrv must be a numeric major.minor version".into());
    }
    if !is_version(&compatibility.framework.pinned_toolchain) {
        errors.push("framework.pinned_toolchain must be a three-part numeric version".into());
    }
    if !is_version(&compatibility.framework.audit_toolchain) {
        errors.push("framework.audit_toolchain must be a three-part numeric version".into());
    }
    if !is_full_sha(&compatibility.framework.previous_release_gpui_rev) {
        errors.push("framework.previous_release_gpui_rev must be a full commit SHA".into());
    }
    if !is_full_sha(&compatibility.gpui.rev) {
        errors.push("gpui.rev must be a full commit SHA".into());
    }
    if compatibility.gpui.repository != "https://github.com/BumpyClock/gpui" {
        errors.push("gpui.repository must be https://github.com/BumpyClock/gpui".into());
    }
    if compatibility.gpui.zed_repository != "https://github.com/zed-industries/zed" {
        errors.push("gpui.zed_repository must be https://github.com/zed-industries/zed".into());
    }
    if compatibility.gpui.zed_upstream_base != "unknown"
        && !is_full_sha(&compatibility.gpui.zed_upstream_base)
    {
        errors.push("gpui.zed_upstream_base must be `unknown` or a full commit SHA".into());
    }
    if compatibility.gpui.zed_upstream_base == "unknown"
        && !compatibility
            .release
            .blockers
            .iter()
            .any(|blocker| blocker.contains("upstream base"))
    {
        errors.push("unknown Zed upstream base must have an explicit release blocker".into());
    }

    let mut dependencies = BTreeSet::new();
    let mut registry_packages = BTreeSet::new();
    for package in &compatibility.gpui.packages {
        if !dependencies.insert(package.dependency.as_str()) {
            errors.push(format!(
                "duplicate GPUI dependency key `{}`",
                package.dependency
            ));
        }
        if !registry_packages.insert(package.registry_package.as_str()) {
            errors.push(format!(
                "conflicting aliases target registry package `{}`",
                package.registry_package
            ));
        }
        if package.dependency.trim().is_empty()
            || package.registry_package.trim().is_empty()
            || package.crate_path.trim().is_empty()
        {
            errors.push("GPUI package fields must not be empty".into());
        }
        if !is_version(&package.version) {
            errors.push(format!(
                "{} version must be a three-part numeric version",
                package.dependency
            ));
        }
        if !VALID_REGISTRY_STATUSES.contains(&package.registry_status.as_str()) {
            errors.push(format!(
                "{} has invalid registry status `{}`",
                package.dependency, package.registry_status
            ));
        }
        if package.registry_note.trim().is_empty() {
            errors.push(format!(
                "{} registry_note must not be empty",
                package.dependency
            ));
        }
    }

    if compatibility.release.crates_io_ready
        && (compatibility
            .gpui
            .packages
            .iter()
            .any(|package| package.registry_status != "published")
            || !compatibility.release.blockers.is_empty())
    {
        errors.push(
            "release.crates_io_ready cannot be true while registry packages or blockers remain"
                .into(),
        );
    }

    for platform in &compatibility.platforms {
        for (field, value) in [
            ("build", &platform.build),
            ("unit", &platform.unit),
            ("headless", &platform.headless),
            ("native_runtime", &platform.native_runtime),
            ("renderer", &platform.renderer),
            ("package", &platform.package),
        ] {
            if !VALID_EVIDENCE_STATUSES.contains(&value.as_str()) {
                errors.push(format!(
                    "{} has invalid {field} status `{value}`",
                    platform.name
                ));
            }
        }
        if !VALID_MATURITY.contains(&platform.maturity.as_str()) {
            errors.push(format!(
                "{} has invalid maturity `{}`",
                platform.name, platform.maturity
            ));
        }
        if platform.target.trim().is_empty() || platform.notes.trim().is_empty() {
            errors.push(format!("{} must have target and notes", platform.name));
        }
    }
}

fn validate_root_manifest(
    manifest: &Value,
    compatibility: &Compatibility,
    errors: &mut Vec<String>,
) {
    let workspace = table_at(manifest, &["workspace"], "Cargo.toml [workspace]", errors);
    let Some(workspace) = workspace else {
        return;
    };
    let package = table_at_table(workspace, "package", "[workspace.package]", errors);
    if let Some(package) = package {
        check_string(
            package,
            "version",
            &compatibility.framework.version,
            "workspace package version",
            errors,
        );
        check_string(
            package,
            "rust-version",
            &compatibility.framework.rust_msrv,
            "workspace rust-version",
            errors,
        );
    }
    let dependencies = table_at_table(
        workspace,
        "dependencies",
        "[workspace.dependencies]",
        errors,
    );
    let Some(dependencies) = dependencies else {
        return;
    };
    for package in &compatibility.gpui.packages {
        let Some(value) = dependencies.get(&package.dependency) else {
            errors.push(format!(
                "workspace dependency `{}` is missing",
                package.dependency
            ));
            continue;
        };
        validate_dependency(
            "Cargo.toml [workspace.dependencies]",
            &package.dependency,
            value,
            package,
            compatibility,
            false,
            errors,
        );
    }
}

fn validate_all_manifests(
    root: &Path,
    compatibility: &Compatibility,
    root_manifest_path: &Path,
    errors: &mut Vec<String>,
) {
    let mut manifests = Vec::new();
    collect_manifests(root, &mut manifests, errors);
    for path in manifests {
        let Ok(manifest) = read_toml(&path) else {
            continue;
        };
        visit_dependency_tables(&manifest, "", &mut |section, dependencies| {
            if path == root_manifest_path && section == "workspace.dependencies" {
                return;
            }
            let is_workspace = section == "workspace.dependencies";
            for package in &compatibility.gpui.packages {
                if let Some(value) = dependencies.get(&package.dependency) {
                    let label = format!("{} [{section}]", relative(root, &path));
                    validate_dependency(
                        &label,
                        &package.dependency,
                        value,
                        package,
                        compatibility,
                        !is_workspace,
                        errors,
                    );
                }
            }
        });
    }
}

fn validate_dependency(
    location: &str,
    dependency: &str,
    value: &Value,
    package: &GpuiPackage,
    compatibility: &Compatibility,
    allow_workspace: bool,
    errors: &mut Vec<String>,
) {
    let Some(table) = value.as_table() else {
        errors.push(format!(
            "{location}: GPUI dependency `{dependency}` must use a detailed declaration"
        ));
        return;
    };
    if allow_workspace && table.get("workspace").and_then(Value::as_bool) == Some(true) {
        if table.contains_key("git")
            || table.contains_key("rev")
            || table.contains_key("branch")
            || table.contains_key("version")
            || table.contains_key("package")
        {
            errors.push(format!(
                "{location}: workspace dependency `{dependency}` must not override source identity"
            ));
        }
        return;
    }

    if table.contains_key("branch") || table.contains_key("tag") {
        errors.push(format!(
            "{location}: GPUI dependency `{dependency}` must use only a full `rev`, not branch/tag"
        ));
    }
    match table.get("git").and_then(Value::as_str) {
        Some(git) if git == compatibility.gpui.repository => {}
        Some(git) => errors.push(format!(
            "{location}: GPUI dependency `{dependency}` uses noncanonical Git URL `{git}`"
        )),
        None => errors.push(format!(
            "{location}: GPUI dependency `{dependency}` is missing `git`"
        )),
    }
    match table.get("rev").and_then(Value::as_str) {
        Some(rev) if rev == compatibility.gpui.rev && is_full_sha(rev) => {}
        Some(rev) => errors.push(format!(
            "{location}: GPUI dependency `{dependency}` uses revision `{rev}`, expected `{}`",
            compatibility.gpui.rev
        )),
        None => errors.push(format!(
            "{location}: GPUI dependency `{dependency}` is missing `rev`"
        )),
    }
    let expected_version = format!("={}", package.version);
    match table.get("version").and_then(Value::as_str) {
        Some(version) if version == expected_version => {}
        Some(version) => errors.push(format!(
            "{location}: public GPUI dependency `{dependency}` version `{version}` is not exact `{expected_version}`"
        )),
        None => errors.push(format!(
            "{location}: GPUI dependency `{dependency}` is Git-only; exact `version` is required"
        )),
    }
    let actual_package = table
        .get("package")
        .and_then(Value::as_str)
        .unwrap_or(dependency);
    if normalize_package(actual_package) != normalize_package(&package.registry_package) {
        errors.push(format!(
            "{location}: dependency `{dependency}` aliases registry package `{actual_package}`, expected `{}`",
            package.registry_package
        ));
    }
    let mut actual_features: Vec<_> = table
        .get("features")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let mut expected_features = package.features.clone();
    actual_features.sort();
    expected_features.sort();
    if actual_features != expected_features {
        errors.push(format!(
            "{location}: dependency `{dependency}` features {actual_features:?}, expected {expected_features:?}"
        ));
    }
}

fn validate_lockfile(root: &Path, compatibility: &Compatibility, errors: &mut Vec<String>) {
    let path = root.join("Cargo.lock");
    let lock = match read_toml(&path) {
        Ok(lock) => lock,
        Err(error) => {
            errors.push(error.to_string());
            return;
        }
    };
    let Some(entries) = lock.get("package").and_then(Value::as_array) else {
        errors.push("Cargo.lock has no package entries".into());
        return;
    };
    let canonical_repository = normalized_git_repository(&compatibility.gpui.repository);
    let canonical_sources: Vec<_> = entries
        .iter()
        .filter_map(Value::as_table)
        .filter_map(|package| {
            let source = package.get("source").and_then(Value::as_str)?;
            (normalized_git_repository(source) == canonical_repository).then_some((package, source))
        })
        .collect();
    for (package, source) in &canonical_sources {
        if resolved_git_revision(source) != Some(compatibility.gpui.rev.as_str()) {
            errors.push(format!(
                "Cargo.lock package `{}` uses canonical GPUI source at revision other than {}: `{source}`",
                package
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("<missing>"),
                compatibility.gpui.rev
            ));
        }
    }
    let canonical_names: BTreeSet<_> = canonical_sources
        .iter()
        .filter_map(|(package, _)| package.get("name").and_then(Value::as_str))
        .map(normalize_package)
        .collect();
    for name in canonical_names {
        let sources: BTreeSet<_> = entries
            .iter()
            .filter_map(Value::as_table)
            .filter(|package| {
                package
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|package_name| normalize_package(package_name) == name)
            })
            .filter_map(|package| package.get("source").and_then(Value::as_str))
            .collect();
        if sources.len() > 1 {
            errors.push(format!(
                "Cargo.lock contains duplicate engine package `{name}` from conflicting sources: {}",
                sources.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    for expected in &compatibility.gpui.packages {
        let matches: Vec<_> = entries
            .iter()
            .filter_map(Value::as_table)
            .filter(|package| {
                package.get("name").and_then(Value::as_str)
                    == Some(expected.registry_package.as_str())
                    && package
                        .get("source")
                        .and_then(Value::as_str)
                        .is_some_and(|source| {
                            normalized_git_repository(source) == canonical_repository
                                && resolved_git_revision(source)
                                    == Some(compatibility.gpui.rev.as_str())
                        })
            })
            .collect();
        if matches.len() != 1 {
            errors.push(format!(
                "Cargo.lock must resolve exactly one Git package `{}` at {}; found {}",
                expected.registry_package,
                compatibility.gpui.rev,
                matches.len()
            ));
            continue;
        }
        let actual_version = matches[0].get("version").and_then(Value::as_str);
        if actual_version != Some(expected.version.as_str()) {
            errors.push(format!(
                "Git package `{}` declares version `{}`, expected `{}`",
                expected.registry_package,
                actual_version.unwrap_or("<missing>"),
                expected.version
            ));
        }
    }
}

fn validate_toolchain(root: &Path, compatibility: &Compatibility, errors: &mut Vec<String>) {
    let path = root.join("rust-toolchain.toml");
    match read_toml(&path) {
        Ok(toolchain) => {
            let actual = value_string(&toolchain, &["toolchain", "channel"]);
            if actual != Some(compatibility.framework.pinned_toolchain.as_str()) {
                errors.push(format!(
                    "rust-toolchain.toml channel is `{}`, expected compatibility pinned_toolchain `{}`",
                    actual.unwrap_or("<missing>"),
                    compatibility.framework.pinned_toolchain
                ));
            }
        }
        Err(error) => errors.push(error.to_string()),
    }
}

fn validate_publishable_dependencies(root: &Path, errors: &mut Vec<String>) {
    let metadata = match cargo_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) => {
            errors.push(error.to_string());
            return;
        }
    };
    validate_publishable_metadata(&metadata, errors);
}

fn validate_publishable_metadata(metadata: &CargoMetadata, errors: &mut Vec<String>) {
    let workspace: BTreeMap<_, _> = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| (normalize_package(&package.name), package))
        .collect();
    for package in workspace.values().filter(|package| is_publishable(package)) {
        for dependency in package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
        {
            if dependency
                .source
                .as_deref()
                .is_some_and(|source| source.starts_with("git+") && dependency.req == "*")
            {
                errors.push(format!(
                    "publishable package `{}` has Git-only normal dependency `{}`",
                    package.name, dependency.name
                ));
            }
            if dependency.path.is_some() && dependency.req == "*" {
                errors.push(format!(
                    "publishable package `{}` has path-only normal dependency `{}`",
                    package.name, dependency.name
                ));
            }
            if dependency.path.is_some()
                && workspace
                    .get(&normalize_package(&dependency.name))
                    .is_some_and(|dependency_package| !is_publishable(dependency_package))
            {
                errors.push(format!(
                    "publishable package `{}` depends on private workspace package `{}`",
                    package.name, dependency.name
                ));
            }
        }
    }
}

fn validate_gpui_checkout(
    gpui_path: &Path,
    compatibility: &Compatibility,
    report: &mut Validation,
) {
    if !gpui_path.join("Cargo.toml").is_file() {
        report.errors.push(format!(
            "--gpui-path {} has no Cargo.toml",
            gpui_path.display()
        ));
        return;
    }
    match command_output(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(gpui_path),
    ) {
        Ok(head) if head.trim() == compatibility.gpui.rev => {}
        Ok(head) => report.warnings.push(format!(
            "local GPUI checkout is a development override at {}, not release-compatible pin {}",
            head.trim(),
            compatibility.gpui.rev
        )),
        Err(error) => report.errors.push(error.to_string()),
    }
    match command_output(
        Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(gpui_path),
    ) {
        Ok(status) if status.trim().is_empty() => {}
        Ok(_) => report
            .warnings
            .push("local GPUI checkout has uncommitted changes and is development-only".into()),
        Err(error) => report.errors.push(error.to_string()),
    }

    for package in &compatibility.gpui.packages {
        let path = gpui_path.join(&package.crate_path).join("Cargo.toml");
        match read_toml(&path) {
            Ok(manifest) => {
                let actual_name = value_string(&manifest, &["package", "name"]);
                let actual_version = value_string(&manifest, &["package", "version"]);
                if actual_name != Some(package.registry_package.as_str()) {
                    report.errors.push(format!(
                        "{} package name is `{}`, expected `{}`",
                        relative(gpui_path, &path),
                        actual_name.unwrap_or("<missing>"),
                        package.registry_package
                    ));
                }
                if actual_version != Some(package.version.as_str()) {
                    report.errors.push(format!(
                        "{} package version is `{}`, expected `{}`",
                        relative(gpui_path, &path),
                        actual_version.unwrap_or("<missing>"),
                        package.version
                    ));
                }
            }
            Err(error) => report.errors.push(error.to_string()),
        }
    }

    let fork_path = gpui_path.join("fork.toml");
    if fork_path.is_file() {
        match read_toml(&fork_path) {
            Ok(fork) => {
                let recorded = value_string(&fork, &["upstream-base-commit"])
                    .or_else(|| value_string(&fork, &["upstream", "base_commit"]))
                    .or_else(|| value_string(&fork, &["upstream", "commit"]));
                if compatibility.gpui.zed_upstream_base != "unknown"
                    && recorded != Some(compatibility.gpui.zed_upstream_base.as_str())
                {
                    report.errors.push(format!(
                        "fork.toml Zed upstream base `{}` does not match compatibility metadata `{}`",
                        recorded.unwrap_or("<missing>"),
                        compatibility.gpui.zed_upstream_base
                    ));
                }
            }
            Err(error) => report.errors.push(error.to_string()),
        }
    } else {
        report
            .errors
            .push(format!("{} is missing fork.toml", gpui_path.display()));
    }
    validate_root_patches(gpui_path, report);
}

fn validate_root_patches(gpui_path: &Path, report: &mut Validation) {
    let manifest = match read_toml(&gpui_path.join("Cargo.toml")) {
        Ok(manifest) => manifest,
        Err(error) => {
            report.errors.push(error.to_string());
            return;
        }
    };
    let metadata = match cargo_metadata_full(gpui_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            report.errors.push(error.to_string());
            return;
        }
    };
    let patches: BTreeSet<_> = resolved_git_root_patches(&manifest, &metadata)
        .into_values()
        .collect();
    for package in patches {
        report.errors.push(format!(
            "GPUI workspace root patch for `{package}` affects the publishable dependency graph but is not inherited by Git consumers or packaged manifests"
        ));
    }
}

fn render(compatibility: &Compatibility) -> String {
    let mut output = String::new();
    output.push_str(
        "---\ntitle: \"Compatibility Matrix\"\nsummary: \"Generated framework, GPUI, registry, and platform compatibility evidence.\"\nread_when: \"updating GPUI pins, packaging crates, or preparing a release\"\n---\n<!-- @generated by `cargo xtask compatibility generate`; do not edit manually. -->\n\n",
    );
    output.push_str("# Compatibility\n\n");
    output.push_str(&format!(
        "- Framework: `{}` `{}`\n- Framework repository: `{}`\n- Declared Rust MSRV: `{}` (not exercised by this audit)\n- Repository-pinned toolchain: `{}` (not installed in this environment)\n- Audit host toolchain: `{}`\n- GPUI repository: `{}`\n- GPUI commit: `{}`\n- Zed upstream: `{}`\n- Zed upstream base: `{}`\n\n",
        compatibility.framework.name,
        compatibility.framework.version,
        compatibility.framework.repository,
        compatibility.framework.rust_msrv,
        compatibility.framework.pinned_toolchain,
        compatibility.framework.audit_toolchain,
        compatibility.gpui.repository,
        compatibility.gpui.rev,
        compatibility.gpui.zed_repository,
        compatibility.gpui.zed_upstream_base,
    ));
    output.push_str("## Engine packages\n\n");
    output.push_str(
        "| Dependency | Registry package | Exact version | Features | Public API | Registry status | Note |\n",
    );
    output.push_str("|---|---|---:|---|:---:|---|---|\n");
    for package in &compatibility.gpui.packages {
        output.push_str(&format!(
            "| `{}` | `{}` | `={}` | {} | {} | {} | {} |\n",
            package.dependency,
            package.registry_package,
            package.version,
            if package.features.is_empty() {
                "none".into()
            } else {
                package
                    .features
                    .iter()
                    .map(|feature| format!("`{feature}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            if package.public_api { "yes" } else { "no" },
            package.registry_status,
            package.registry_note
        ));
    }
    output.push_str("\n## Consumption modes\n\n");
    output.push_str(
        "**Git source.** Select an immutable `gpui-component` tag or commit. Its source manifest uses the exact GPUI Git revision and the matching exact registry version. Do not select a separate GPUI revision.\n\n",
    );
    output.push_str(
        "**crates.io.** Once registry readiness is true, select the published framework version. Cargo's normalized package manifests omit the Git location and resolve the same exact engine versions from crates.io. Git and registry sources must represent identical engine source and public behavior.\n\n",
    );
    output.push_str("## Platform evidence\n\n");
    output.push_str("| Platform | Target | Build | Unit | Headless | Native runtime | Renderer presentation | Package artifact | Maturity |\n");
    output.push_str("|---|---|---|---|---|---|---|---|---|\n");
    for platform in &compatibility.platforms {
        output.push_str(&format!(
            "| {} | `{}` | {} | {} | {} | {} | {} | {} | {} |\n",
            platform.name,
            platform.target,
            platform.build,
            platform.unit,
            platform.headless,
            platform.native_runtime,
            platform.renderer,
            platform.package,
            platform.maturity
        ));
    }
    output.push_str("\n");
    for platform in &compatibility.platforms {
        output.push_str(&format!("- **{}:** {}\n", platform.name, platform.notes));
    }
    output.push_str("\n## crates.io readiness\n\n");
    output.push_str(if compatibility.release.crates_io_ready {
        "**Ready.** Compatibility and package checks still must pass for the release commit.\n"
    } else {
        "**Blocked.** Framework publication must not start yet.\n"
    });
    if !compatibility.release.blockers.is_empty() {
        output.push_str("\nBlockers:\n\n");
        for blocker in &compatibility.release.blockers {
            output.push_str(&format!("- {blocker}\n"));
        }
    }
    output.push_str(&format!(
        "\nHistorical evidence: immutable `v{}` pinned GPUI `{}`. Current `{}` metadata defines a new compatibility line; it does not reinterpret that tag.\n",
        compatibility.framework.previous_release,
        compatibility.framework.previous_release_gpui_rev,
        compatibility.framework.version
    ));
    output
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    workspace_members: Vec<String>,
    #[serde(default)]
    resolve: Option<CargoResolve>,
}

#[derive(Debug, Deserialize)]
struct CargoResolve {
    nodes: Vec<CargoNode>,
}

#[derive(Debug, Deserialize)]
struct CargoNode {
    id: String,
    deps: Vec<CargoNodeDependency>,
}

#[derive(Debug, Deserialize)]
struct CargoNodeDependency {
    pkg: String,
    #[serde(default)]
    dep_kinds: Vec<CargoNodeDependencyKind>,
}

#[derive(Debug, Deserialize)]
struct CargoNodeDependencyKind {
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    version: String,
    source: Option<String>,
    publish: Option<Vec<String>>,
    description: Option<String>,
    license: Option<String>,
    license_file: Option<PathBuf>,
    readme: Option<PathBuf>,
    repository: Option<String>,
    rust_version: Option<String>,
    dependencies: Vec<CargoDependency>,
}

#[derive(Debug, Deserialize)]
struct CargoDependency {
    name: String,
    source: Option<String>,
    req: String,
    path: Option<PathBuf>,
    kind: Option<String>,
}

#[derive(Clone)]
struct PlanNode {
    id: String,
    repository: String,
    package: String,
    registry_package: String,
    version: String,
    prerequisites: BTreeSet<String>,
    metadata_ready: bool,
    registry_status: String,
    full_dry_run: String,
    non_registry_blocker: bool,
}

fn publish_plan(root: &Path, gpui_path: Option<&Path>, require_registry: bool) -> Result<()> {
    let compatibility = load(root)?;
    let nodes = build_plan(root, gpui_path, &compatibility)?;
    println!(
        "repository\tpackage\tregistry package\tversion\tregistry\tprerequisites\tmetadata\tfull dry-run"
    );
    for node in &nodes {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            node.repository,
            node.package,
            node.registry_package,
            node.version,
            node.registry_status,
            if node.prerequisites.is_empty() {
                "-".into()
            } else {
                node.prerequisites
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(",")
            },
            if node.metadata_ready {
                "ready"
            } else {
                "blocked"
            },
            node.full_dry_run
        );
    }
    if require_registry {
        require_registry_gate(&nodes)?;
    }
    Ok(())
}

fn require_registry_gate(nodes: &[PlanNode]) -> Result<()> {
    let blocked: Vec<_> = nodes
        .iter()
        .filter(|node| node.registry_status != "published" || node.non_registry_blocker)
        .map(|node| {
            let mut reasons = Vec::new();
            if node.registry_status != "published" {
                reasons.push(format!("registry={}", node.registry_status));
            }
            if node.non_registry_blocker {
                reasons.push(node.full_dry_run.clone());
            }
            format!(
                "{}/{} ={} ({})",
                node.repository,
                node.registry_package,
                node.version,
                reasons.join("; ")
            )
        })
        .collect();
    if !blocked.is_empty() {
        bail!(
            "publication gate blocked:\n{}",
            blocked
                .into_iter()
                .map(|package| format!("- {package}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    Ok(())
}

fn build_plan(
    root: &Path,
    gpui_path: Option<&Path>,
    compatibility: &Compatibility,
) -> Result<Vec<PlanNode>> {
    let mut nodes = BTreeMap::<String, PlanNode>::new();
    if let Some(gpui_path) = gpui_path {
        let metadata = cargo_metadata_full(gpui_path)?;
        let manifest = read_toml(&gpui_path.join("Cargo.toml"))?;
        let root_patches = resolved_git_root_patches(&manifest, &metadata);
        let patch_reachability = root_patch_reachability(&metadata, &root_patches)?;
        let fork_registry = fork_registry_statuses(gpui_path)?;
        add_workspace_nodes(&metadata, "BumpyClock/gpui", &mut nodes, |package| {
            compatibility
                .gpui
                .packages
                .iter()
                .find(|item| {
                    normalize_package(&item.registry_package) == normalize_package(&package.name)
                })
                .map(|item| item.registry_status.clone())
                .or_else(|| {
                    fork_registry
                        .get(&normalize_package(&package.name))
                        .cloned()
                })
                .unwrap_or_else(|| "unknown".into())
        })?;
        for (package, patches) in patch_reachability {
            let Some(node) = nodes.get_mut(&plan_id("BumpyClock/gpui", &package)) else {
                continue;
            };
            node.full_dry_run = format!(
                "blocked: non-inherited GPUI root patches: {}",
                patches.into_iter().collect::<Vec<_>>().join(", ")
            );
            node.non_registry_blocker = true;
        }
    } else {
        let resolved = cargo_metadata_full(root)?;
        add_resolved_engine_nodes(&resolved, compatibility, &mut nodes);
    }

    let framework = cargo_metadata(root)?;
    let framework_registry: BTreeMap<_, _> = framework
        .packages
        .iter()
        .filter(|package| framework.workspace_members.contains(&package.id))
        .filter(|package| is_publishable(package))
        .map(|package| {
            (
                normalize_package(&package.name),
                registry_probe(&package.name, &package.version),
            )
        })
        .collect();
    add_workspace_nodes(
        &framework,
        "BumpyClock/gpui-component",
        &mut nodes,
        |package| {
            framework_registry
                .get(&normalize_package(&package.name))
                .cloned()
                .unwrap_or_else(|| "unknown".into())
        },
    )?;

    let engine_ids: BTreeMap<_, _> = compatibility
        .gpui
        .packages
        .iter()
        .map(|package| {
            (
                normalize_package(&package.registry_package),
                plan_id("BumpyClock/gpui", &package.registry_package),
            )
        })
        .collect();
    for package in &framework.packages {
        let id = plan_id("BumpyClock/gpui-component", &package.name);
        let Some(node) = nodes.get_mut(&id) else {
            continue;
        };
        let mut directly_uses_engine = false;
        for dependency in package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
        {
            if let Some(engine_id) = engine_ids.get(&normalize_package(&dependency.name)) {
                node.prerequisites.insert(engine_id.clone());
                directly_uses_engine = true;
            }
        }
        if directly_uses_engine
            && compatibility
                .gpui
                .packages
                .iter()
                .any(|package| package.registry_status != "published")
        {
            node.full_dry_run = "blocked: exact engine packages are not published".into();
        }
    }
    topological_sort(nodes)
}

fn fork_registry_statuses(gpui_path: &Path) -> Result<BTreeMap<String, String>> {
    let fork = read_toml(&gpui_path.join("fork.toml"))?;
    Ok(fork
        .get("registry-packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_table)
        .filter_map(|package| {
            let identity = package
                .get("package")
                .and_then(Value::as_str)
                .or_else(|| package.get("workspace").and_then(Value::as_str))?;
            Some((
                normalize_package(identity),
                package.get("status")?.as_str()?.to_owned(),
            ))
        })
        .collect())
}

fn add_resolved_engine_nodes(
    metadata: &CargoMetadata,
    compatibility: &Compatibility,
    nodes: &mut BTreeMap<String, PlanNode>,
) {
    let engine_packages: BTreeMap<_, _> = metadata
        .packages
        .iter()
        .filter(|package| {
            package.source.as_deref().is_some_and(|source| {
                normalized_git_repository(source)
                    == normalized_git_repository(&compatibility.gpui.repository)
                    && resolved_git_revision(source) == Some(compatibility.gpui.rev.as_str())
            })
        })
        .map(|package| (normalize_package(&package.name), package))
        .collect();
    let mut reachable = BTreeSet::new();
    let mut pending: VecDeque<_> = compatibility
        .gpui
        .packages
        .iter()
        .map(|package| normalize_package(&package.registry_package))
        .collect();
    while let Some(name) = pending.pop_front() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        let Some(package) = engine_packages.get(&name) else {
            continue;
        };
        for dependency in package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
        {
            let dependency = normalize_package(&dependency.name);
            if engine_packages.contains_key(&dependency) {
                pending.push_back(dependency);
            }
        }
    }
    for package in engine_packages
        .values()
        .filter(|package| reachable.contains(&normalize_package(&package.name)))
    {
        let status = compatibility
            .gpui
            .packages
            .iter()
            .find(|item| {
                normalize_package(&item.registry_package) == normalize_package(&package.name)
            })
            .map(|item| item.registry_status.clone())
            .unwrap_or_else(|| {
                if is_publishable(package) {
                    "unavailable".into()
                } else {
                    "private".into()
                }
            });
        let id = plan_id("BumpyClock/gpui", &package.name);
        let prerequisites = package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
            .filter_map(|dependency| {
                engine_packages
                    .get(&normalize_package(&dependency.name))
                    .map(|prerequisite| plan_id("BumpyClock/gpui", &prerequisite.name))
            })
            .collect();
        nodes.insert(
            id.clone(),
            PlanNode {
                id,
                repository: "BumpyClock/gpui".into(),
                package: package.name.clone(),
                registry_package: package.name.clone(),
                version: package.version.clone(),
                prerequisites,
                metadata_ready: is_publishable(package)
                    && package_metadata_ready(package)
                    && status != "conflict",
                registry_status: status.clone(),
                full_dry_run: match status.as_str() {
                    "published" => "possible".into(),
                    "private" => "blocked: required engine package is private".into(),
                    _ => "blocked: engine registry prerequisite unavailable".into(),
                },
                non_registry_blocker: false,
            },
        );
    }
    for package in &compatibility.gpui.packages {
        let id = plan_id("BumpyClock/gpui", &package.registry_package);
        nodes.entry(id.clone()).or_insert_with(|| PlanNode {
            id,
            repository: "BumpyClock/gpui".into(),
            package: package.registry_package.clone(),
            registry_package: package.registry_package.clone(),
            version: package.version.clone(),
            prerequisites: BTreeSet::new(),
            metadata_ready: false,
            registry_status: package.registry_status.clone(),
            full_dry_run: "blocked: package is not publishable at the pinned GPUI commit".into(),
            non_registry_blocker: true,
        });
    }
}

fn resolved_git_root_patches(
    manifest: &Value,
    metadata: &CargoMetadata,
) -> BTreeMap<String, String> {
    let patches: BTreeMap<_, _> = manifest
        .get("patch")
        .and_then(|patch| patch.get("crates-io"))
        .and_then(Value::as_table)
        .into_iter()
        .flatten()
        .filter_map(|(package, source)| {
            Some((
                normalize_package(package),
                (
                    package.clone(),
                    normalized_git_repository(source.as_table()?.get("git")?.as_str()?),
                ),
            ))
        })
        .collect();
    metadata
        .packages
        .iter()
        .filter_map(|package| {
            let (patch_name, patch_repository) = patches.get(&normalize_package(&package.name))?;
            let source = package.source.as_deref()?;
            (normalized_git_repository(source) == *patch_repository)
                .then(|| (package.id.clone(), patch_name.clone()))
        })
        .collect()
}

fn normalized_git_repository(source: &str) -> String {
    source
        .strip_prefix("git+")
        .unwrap_or(source)
        .split(['?', '#'])
        .next()
        .unwrap_or(source)
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_owned()
}

fn resolved_git_revision(source: &str) -> Option<&str> {
    source.rsplit_once('#').map(|(_, revision)| revision)
}

fn root_patch_reachability(
    metadata: &CargoMetadata,
    root_patches: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let resolve = metadata
        .resolve
        .as_ref()
        .context("full cargo metadata did not include a resolve graph")?;
    let dependencies: BTreeMap<_, _> = resolve
        .nodes
        .iter()
        .map(|node| {
            (
                node.id.as_str(),
                node.deps
                    .iter()
                    .filter(|dependency| {
                        dependency.dep_kinds.is_empty()
                            || dependency
                                .dep_kinds
                                .iter()
                                .any(|kind| kind.kind.as_deref() != Some("dev"))
                    })
                    .map(|dependency| dependency.pkg.as_str())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let members: BTreeSet<_> = metadata.workspace_members.iter().collect();
    let mut reachable = BTreeMap::new();
    for package in metadata
        .packages
        .iter()
        .filter(|package| members.contains(&package.id) && is_publishable(package))
    {
        let mut patches = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut pending = VecDeque::from([package.id.as_str()]);
        while let Some(id) = pending.pop_front() {
            if !visited.insert(id) {
                continue;
            }
            if let Some(patch) = root_patches.get(id) {
                patches.insert(patch.clone());
            }
            if let Some(dependencies) = dependencies.get(id) {
                pending.extend(dependencies.iter().copied());
            }
        }
        if !patches.is_empty() {
            reachable.insert(normalize_package(&package.name), patches);
        }
    }
    Ok(reachable)
}

fn add_workspace_nodes(
    metadata: &CargoMetadata,
    repository: &str,
    nodes: &mut BTreeMap<String, PlanNode>,
    registry_status: impl Fn(&CargoPackage) -> String,
) -> Result<()> {
    let members: BTreeSet<_> = metadata.workspace_members.iter().collect();
    let publishable: BTreeMap<_, _> = metadata
        .packages
        .iter()
        .filter(|package| members.contains(&package.id) && is_publishable(package))
        .map(|package| (normalize_package(&package.name), package))
        .collect();
    for package in publishable.values() {
        let status = registry_status(package);
        let id = plan_id(repository, &package.name);
        let metadata_ready = package_metadata_ready(package);
        let prerequisites = package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
            .filter_map(|dependency| {
                publishable
                    .get(&normalize_package(&dependency.name))
                    .map(|prerequisite| plan_id(repository, &prerequisite.name))
            })
            .collect();
        nodes.insert(
            id.clone(),
            PlanNode {
                id,
                repository: repository.into(),
                package: package.name.clone(),
                registry_package: package.name.clone(),
                version: package.version.clone(),
                prerequisites,
                metadata_ready,
                registry_status: status,
                full_dry_run: if metadata_ready {
                    "possible after registry prerequisites".into()
                } else {
                    "blocked: package metadata incomplete".into()
                },
                non_registry_blocker: !metadata_ready,
            },
        );
    }
    Ok(())
}

fn topological_sort(mut nodes: BTreeMap<String, PlanNode>) -> Result<Vec<PlanNode>> {
    let mut sorted = Vec::new();
    let known: BTreeSet<_> = nodes.keys().cloned().collect();
    for node in nodes.values_mut() {
        node.prerequisites.retain(|item| known.contains(item));
    }
    while !nodes.is_empty() {
        let next = nodes
            .iter()
            .filter(|(_, node)| {
                node.prerequisites.iter().all(|prerequisite| {
                    sorted
                        .iter()
                        .any(|done: &PlanNode| &done.id == prerequisite)
                })
            })
            .min_by_key(|(_, node)| (publication_phase(node), node.id.as_str()))
            .map(|(id, _)| id.clone());
        let Some(next) = next else {
            bail!("publication graph contains a cycle");
        };
        sorted.push(nodes.remove(&next).expect("selected node exists"));
    }
    Ok(sorted)
}

fn publication_phase(node: &PlanNode) -> u8 {
    match (node.repository.as_str(), node.package.as_str()) {
        ("BumpyClock/gpui", _) => 0,
        ("BumpyClock/gpui-component", "gpui-component") => 2,
        _ => 1,
    }
}

fn cargo_headless_test_args() -> [&'static str; 8] {
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
}

fn release_check(root: &Path, options: &Options) -> Result<()> {
    println!("1/5 compatibility metadata and generated documentation");
    check(root, options.gpui_path.as_deref())?;

    let compatibility = load(root)?;
    println!("2/5 source build");
    run(Command::new("cargo")
        .args(["check", "--locked", "--workspace", "--all-targets"])
        .current_dir(root))?;

    println!("3/5 unit and headless tests");
    run(Command::new("cargo")
        .args(["test", "--locked", "--workspace", "--all-targets"])
        .current_dir(root))?;
    run(Command::new("cargo")
        .args(cargo_headless_test_args())
        .current_dir(root))?;

    println!("4/5 publication plan");
    publish_plan(root, options.gpui_path.as_deref(), options.require_registry)?;

    println!("5/5 package file lists and normalized manifests");
    let blockers = validate_packages(root, &compatibility, options.require_registry)?;
    if blockers.is_empty() {
        if options.require_registry {
            println!("code, manifest, package, and registry release checks passed");
        } else {
            println!(
                "code and manifest checks passed; registry readiness was not required (run with --require-registry for publication)"
            );
        }
    } else {
        println!("code checks passed; registry-dependent package artifacts remain blocked:");
        for blocker in blockers {
            println!("- {blocker}");
        }
    }
    Ok(())
}

fn validate_packages(
    root: &Path,
    compatibility: &Compatibility,
    require_registry: bool,
) -> Result<Vec<String>> {
    let metadata = cargo_metadata(root)?;
    let mut blockers = Vec::new();
    let engine_registry_blocked = compatibility
        .gpui
        .packages
        .iter()
        .any(|package| package.registry_status != "published");
    for package in publishable_package_order(&metadata)? {
        run(Command::new("cargo")
            .args(cargo_package_args(
                &package.name,
                true,
                !require_registry,
                false,
            ))
            .current_dir(root))?;
        let mut command = Command::new("cargo");
        command
            .args(cargo_package_args(
                &package.name,
                false,
                !require_registry,
                !require_registry,
            ))
            .current_dir(root);
        let output = command
            .output()
            .with_context(|| format!("failed to package {}", package.name))?;
        if output.status.success() {
            inspect_package_files(root, package)?;
            inspect_normalized_manifest(root, package, compatibility)?;
            continue;
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !require_registry
            && engine_registry_blocked
            && let Some(registry_failure) =
                unavailable_engine_registry_failure(&stderr, compatibility)
        {
            blockers.push(format!(
                "{} {} normalized artifact blocked until exact engine/framework prerequisites are published: {}",
                package.name,
                package.version,
                registry_failure.trim()
            ));
            continue;
        }
        bail!(
            "cargo package failed for {} {}:\n{}",
            package.name,
            package.version,
            stderr
        );
    }
    Ok(blockers)
}

fn cargo_package_args(
    package: &str,
    list: bool,
    allow_dirty: bool,
    no_verify: bool,
) -> Vec<String> {
    let mut args = vec!["package".into(), "--locked".into()];
    if list {
        args.push("--list".into());
    }
    if allow_dirty {
        args.push("--allow-dirty".into());
    }
    args.extend(["-p".into(), package.into()]);
    if no_verify {
        args.push("--no-verify".into());
    }
    args
}

fn unavailable_engine_registry_failure<'a>(
    stderr: &'a str,
    compatibility: &Compatibility,
) -> Option<&'a str> {
    for package in compatibility
        .gpui
        .packages
        .iter()
        .filter(|package| package.registry_status != "published")
    {
        let exact_requirement =
            format!("`{} = \"={}\"`", package.registry_package, package.version);
        let missing_package = format!("`{}` found", package.registry_package);
        if let Some(line) = stderr.lines().find(|line| {
            (line.contains("failed to select a version") && line.contains(&exact_requirement))
                || (line.contains("no matching package named") && line.contains(&missing_package))
        }) {
            return Some(line);
        }
    }
    None
}

fn publishable_package_order(metadata: &CargoMetadata) -> Result<Vec<&CargoPackage>> {
    let mut remaining: BTreeMap<_, _> = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .filter(|package| is_publishable(package))
        .map(|package| (normalize_package(&package.name), package))
        .collect();
    let package_names: BTreeSet<_> = remaining.keys().cloned().collect();
    let mut published = BTreeSet::new();
    let mut ordered = Vec::new();
    while !remaining.is_empty() {
        let next = remaining
            .iter()
            .find(|(_, package)| {
                package
                    .dependencies
                    .iter()
                    .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
                    .map(|dependency| normalize_package(&dependency.name))
                    .filter(|dependency| package_names.contains(dependency))
                    .all(|dependency| published.contains(&dependency))
            })
            .map(|(name, _)| name.clone())
            .context("publishable framework package graph contains a cycle")?;
        let package = remaining.remove(&next).expect("selected package exists");
        published.insert(next);
        ordered.push(package);
    }
    Ok(ordered)
}

fn inspect_package_files(root: &Path, package: &CargoPackage) -> Result<()> {
    let archive = root
        .join("target/package")
        .join(format!("{}-{}.crate", package.name, package.version));
    let listing = command_output(
        Command::new("tar").args([
            "-tzf",
            archive
                .to_str()
                .context("package archive path is not UTF-8")?,
        ]),
    )?;
    let files: Vec<_> = listing.lines().collect();
    if !files.iter().any(|file| {
        file.rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with("README"))
    }) {
        bail!("{} contains no README file", archive.display());
    }
    if !files.iter().any(|file| {
        file.rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with("LICENSE"))
    }) {
        bail!("{} contains no LICENSE file", archive.display());
    }
    Ok(())
}

fn inspect_normalized_manifest(
    root: &Path,
    package: &CargoPackage,
    compatibility: &Compatibility,
) -> Result<()> {
    let archive = root
        .join("target/package")
        .join(format!("{}-{}.crate", package.name, package.version));
    let member = format!("{}-{}/Cargo.toml", package.name, package.version);
    let source = command_output(
        Command::new("tar").args([
            "-xOf",
            archive
                .to_str()
                .context("package archive path is not UTF-8")?,
            &member,
        ]),
    )?;
    let manifest: Value = toml::from_str(&source)
        .with_context(|| format!("invalid normalized manifest in {}", archive.display()))?;
    let mut errors = Vec::new();
    visit_dependency_tables(&manifest, "", &mut |section, dependencies| {
        if !section.ends_with("dependencies") || section.ends_with("dev-dependencies") {
            return;
        }
        for (name, value) in dependencies {
            let Some(table) = value.as_table() else {
                continue;
            };
            if table.contains_key("git") {
                errors.push(format!(
                    "{} [{section}] dependency `{name}` retains a Git source",
                    archive.display()
                ));
            }
            if table.get("path").is_some() && table.get("version").is_none() {
                errors.push(format!(
                    "{} [{section}] dependency `{name}` is path-only",
                    archive.display()
                ));
            }
            if let Some(expected) = compatibility
                .gpui
                .packages
                .iter()
                .find(|expected| expected.dependency == *name)
            {
                let version = table.get("version").and_then(Value::as_str);
                if version != Some(format!("={}", expected.version).as_str()) {
                    errors.push(format!(
                        "{} normalized `{name}` version is `{}`, expected `={}`",
                        archive.display(),
                        version.unwrap_or("<missing>"),
                        expected.version
                    ));
                }
                let registry_package = table.get("package").and_then(Value::as_str).unwrap_or(name);
                if normalize_package(registry_package)
                    != normalize_package(&expected.registry_package)
                {
                    errors.push(format!(
                        "{} normalized `{name}` package is `{registry_package}`, expected `{}`",
                        archive.display(),
                        expected.registry_package
                    ));
                }
                let mut actual_features: Vec<_> = table
                    .get("features")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                let mut expected_features = expected.features.clone();
                actual_features.sort();
                expected_features.sort();
                if actual_features != expected_features {
                    errors.push(format!(
                        "{} normalized `{name}` features {actual_features:?}, expected {expected_features:?}",
                        archive.display()
                    ));
                }
            }
        }
    });
    if errors.is_empty() {
        Ok(())
    } else {
        Err(validation_error(errors))
    }
}

fn cargo_metadata(root: &Path) -> Result<CargoMetadata> {
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to run cargo metadata in {}", root.display()))?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed in {}:\n{}",
            root.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).context("cargo metadata returned invalid JSON")
}

fn cargo_metadata_full(root: &Path) -> Result<CargoMetadata> {
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--format-version", "1"])
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to run cargo metadata in {}", root.display()))?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed in {}:\n{}",
            root.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).context("cargo metadata returned invalid JSON")
}

fn registry_probe(package: &str, version: &str) -> String {
    let mut child = match Command::new("cargo")
        .args([
            "info",
            &format!("{package}@{version}"),
            "--registry",
            "crates-io",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return "unknown".into(),
    };
    let output = match child.wait_timeout(REGISTRY_PROBE_TIMEOUT) {
        Ok(Some(_)) => child.wait_with_output().ok(),
        Ok(None) | Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return "unknown".into();
        }
    };
    match output {
        Some(output) => registry_probe_status(output.status.success(), &output.stderr).into(),
        None => "unknown".into(),
    }
}

fn registry_probe_status(success: bool, stderr: &[u8]) -> &'static str {
    if success {
        return "published";
    }
    let stderr = String::from_utf8_lossy(stderr);
    if stderr.contains("could not find") || stderr.contains("no matching package") {
        "unpublished"
    } else {
        "unknown"
    }
}

fn package_metadata_ready(package: &CargoPackage) -> bool {
    package.description.is_some()
        && (package.license.is_some() || package.license_file.is_some())
        && package.readme.is_some()
        && package.repository.is_some()
        && package.rust_version.is_some()
}

fn is_publishable(package: &CargoPackage) -> bool {
    !matches!(package.publish.as_deref(), Some([]))
}

fn plan_id(repository: &str, package: &str) -> String {
    format!("{repository}/{}", normalize_package(package))
}

fn read_toml(path: &Path) -> Result<Value> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&source).with_context(|| format!("invalid TOML in {}", path.display()))
}

fn collect_manifests(directory: &Path, manifests: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!("failed to read {}: {error}", directory.display()));
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some(".git" | "target" | "vendor" | "node_modules")
            ) {
                continue;
            }
            collect_manifests(&path, manifests, errors);
        } else if path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml") {
            manifests.push(path);
        }
    }
}

fn visit_dependency_tables(
    value: &Value,
    prefix: &str,
    visitor: &mut impl FnMut(&str, &toml::map::Map<String, Value>),
) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if matches!(
            key.as_str(),
            "dependencies" | "dev-dependencies" | "build-dependencies"
        ) {
            if let Some(dependencies) = value.as_table() {
                visitor(&path, dependencies);
            }
        }
        visit_dependency_tables(value, &path, visitor);
    }
}

fn table_at<'a>(
    value: &'a Value,
    path: &[&str],
    label: &str,
    errors: &mut Vec<String>,
) -> Option<&'a toml::map::Map<String, Value>> {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            errors.push(format!("{label} is missing"));
            return None;
        };
        current = next;
    }
    match current.as_table() {
        Some(table) => Some(table),
        None => {
            errors.push(format!("{label} must be a table"));
            None
        }
    }
}

fn table_at_table<'a>(
    table: &'a toml::map::Map<String, Value>,
    key: &str,
    label: &str,
    errors: &mut Vec<String>,
) -> Option<&'a toml::map::Map<String, Value>> {
    match table.get(key).and_then(Value::as_table) {
        Some(table) => Some(table),
        None => {
            errors.push(format!("{label} is missing or not a table"));
            None
        }
    }
}

fn check_string(
    table: &toml::map::Map<String, Value>,
    key: &str,
    expected: &str,
    label: &str,
    errors: &mut Vec<String>,
) {
    let actual = table.get(key).and_then(Value::as_str);
    if actual != Some(expected) {
        errors.push(format!(
            "{label} is `{}`, expected `{expected}`",
            actual.unwrap_or("<missing>")
        ));
    }
}

fn value_string<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str()
}

fn normalize_package(value: &str) -> String {
    value.replace('-', "_")
}

fn is_full_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_version(value: &str) -> bool {
    value.split('.').count() == 3
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_msrv(value: &str) -> bool {
    value.split('.').count() == 2
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn run(command: &mut Command) -> Result<()> {
    let description = format!("{command:?}");
    let status = command
        .status()
        .with_context(|| format!("failed to run {description}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{description} exited with {status}")
    }
}

fn command_output(command: &mut Command) -> Result<String> {
    let description = format!("{command:?}");
    let output = command
        .output()
        .with_context(|| format!("failed to run {description}"))?;
    if !output.status.success() {
        bail!(
            "{description} exited with {}:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout).context("command output was not UTF-8")
}

#[cfg(test)]
mod tests;
