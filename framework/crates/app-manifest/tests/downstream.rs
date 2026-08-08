use std::process::Command;

#[test]
fn consuming_app_generates_identity_from_its_own_manifest() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/downstream-app");
    let target = tempfile::tempdir().expect("create isolated target directory");
    // The fixture is a separate workspace with no committed Cargo.lock, so cargo
    // resolves its dependency graph fresh here. `--offline`/`--locked` are
    // intentionally omitted: pinning fixture-only versions in a committed
    // lockfile made this test fail on clean CI runners whose cargo cache holds
    // only workspace-locked deps. Network access is permitted for cargo tests on
    // CI, matching every other build in the suite; the regenerated Cargo.lock is
    // gitignored so it never dirties the tree.
    let output = Command::new(env!("CARGO"))
        .args(["run", "--quiet", "--target-dir"])
        .arg(target.path())
        .current_dir(fixture)
        .output()
        .expect("run downstream fixture");

    assert!(
        output.status.success(),
        "fixture failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IDENTITY_OK com.example.downstreamfixture 0.4.2"),
        "unexpected stdout: {stdout:?}"
    );
}
