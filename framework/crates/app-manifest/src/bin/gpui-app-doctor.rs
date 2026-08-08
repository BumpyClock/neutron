//! Read-only packaging identity verifier.

use std::{
    env,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

#[path = "../doctor.rs"]
mod doctor;

fn main() -> ExitCode {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    ExitCode::from(run(env::args().skip(1), &mut stdout, &mut stderr))
}

fn run(
    args: impl IntoIterator<Item = String>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> u8 {
    match parse_args(args) {
        Ok(ParsedArgs::Help) => {
            let _ = writeln!(stdout, "usage: gpui-app-doctor [--json] [PATH]");
            0
        }
        Ok(ParsedArgs::Options(options)) => match doctor::verify(&options.root) {
            Ok(report) => {
                let result = if options.json {
                    write_json(stdout, &report)
                } else {
                    write_table(stdout, &report)
                };
                if let Err(error) = result {
                    let _ = writeln!(stderr, "error: failed to write report: {error}");
                    return 2;
                }
                u8::from(report.has_failures())
            }
            Err(error) => {
                let _ = writeln!(stderr, "error: {error}");
                2
            }
        },
        Err(message) => {
            let _ = writeln!(
                stderr,
                "error: {message}\nusage: gpui-app-doctor [--json] [PATH]"
            );
            2
        }
    }
}

struct Options {
    root: PathBuf,
    json: bool,
}

enum ParsedArgs {
    Help,
    Options(Options),
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<ParsedArgs, String> {
    let mut root = None;
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "-h" | "--help" => return Ok(ParsedArgs::Help),
            value if value.starts_with('-') => return Err(format!("unknown option {value:?}")),
            value if root.is_none() => root = Some(PathBuf::from(value)),
            value => return Err(format!("unexpected argument {value:?}")),
        }
    }
    Ok(ParsedArgs::Options(Options {
        root: root.unwrap_or_else(|| PathBuf::from(".")),
        json,
    }))
}

fn write_table(output: &mut impl Write, report: &doctor::Report) -> io::Result<()> {
    let field_width = report
        .checks
        .iter()
        .map(|row| row.field.len())
        .max()
        .unwrap_or(5)
        .max(5);
    let manifest_width = report
        .checks
        .iter()
        .map(|row| row.manifest_value.len())
        .max()
        .unwrap_or(14)
        .max(14);
    let artifact_width = report
        .checks
        .iter()
        .map(|row| row.artifact_value.len())
        .max()
        .unwrap_or(14)
        .max(14);
    writeln!(
        output,
        "{:<field_width$} | {:<manifest_width$} | {:<artifact_width$} | status",
        "field", "manifest value", "artifact value"
    )?;
    writeln!(
        output,
        "{:-<field_width$}-+-{:-<manifest_width$}-+-{:-<artifact_width$}-+---------",
        "", "", ""
    )?;
    for row in &report.checks {
        writeln!(
            output,
            "{:<field_width$} | {:<manifest_width$} | {:<artifact_width$} | {}",
            row.field,
            row.manifest_value,
            row.artifact_value,
            row.status.as_str()
        )?;
    }
    Ok(())
}

fn write_json(output: &mut impl Write, report: &doctor::Report) -> io::Result<()> {
    writeln!(output, "[")?;
    for (index, row) in report.checks.iter().enumerate() {
        write!(
            output,
            "  {{\"field\":\"{}\",\"manifest_value\":\"{}\",\"artifact_value\":\"{}\",\"status\":\"{}\"}}",
            escape_json(&row.field),
            escape_json(&row.manifest_value),
            escape_json(&row.artifact_value),
            row.status.as_str()
        )?;
        writeln!(
            output,
            "{}",
            if index + 1 == report.checks.len() {
                ""
            } else {
                ","
            }
        )?;
    }
    writeln!(output, "]")
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            value if value <= '\u{1f}' => {
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\u{:04x}", value as u32);
            }
            value => escaped.push(value),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};
    use tempfile::tempdir;

    fn manifest(identifier: &str) -> String {
        format!(
            r#"[package]
name = "example"
[package.metadata.gpui-app]
app_id = "com.example.app"
display_name = "Example"
[package.metadata.bundle]
identifier = "{identifier}"
name = "Example"
icon = "icon.png"
"#
        )
    }

    fn write(root: &Path, path: &str, source: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, source).expect("write fixture");
    }

    fn ansible_fixture(root: &Path) {
        write(
            root,
            "Cargo.toml",
            r#"[package]
name = "ansible"
[package.metadata.gpui-app]
app_id = "com.weakly-design.ansible"
display_name = "Ansible"
binary_name = "ansible"
[package.metadata.gpui-app.macos]
entitlements = ["com.apple.security.device.audio-input"]
[package.metadata.gpui-app.macos.usage_strings]
NSMicrophoneUsageDescription = "Ansible needs microphone access"
[package.metadata.bundle]
identifier = "com.weakly-design.ansible"
name = "Ansible"
icon = ["icon.icns", "icon.png"]
"#,
        );
        write(
            root,
            "Info.plist",
            r#"<plist><dict><key>CFBundleIdentifier</key><string>com.weakly-design.ansible</string><key>CFBundleName</key><string>Ansible</string><key>NSMicrophoneUsageDescription</key><string>Ansible needs microphone access</string></dict></plist>"#,
        );
        write(
            root,
            "entitlements.plist",
            "<plist><dict><key>com.apple.security.device.audio-input</key><true/></dict></plist>",
        );
        write(
            root,
            "packaging/ansible.desktop",
            "[Desktop Entry]\nName=Ansible\nExec=/usr/bin/ansible %U\nIcon=ansible\nStartupWMClass=ansible\n",
        );
        write(
            root,
            "packaging/windows/AppxManifest.xml",
            "<Package><Identity\n Name=\"com.weakly-design.ansible\"\n Publisher=\"CN=Weakly\" /></Package>",
        );
        write(
            root,
            "packaging/linux/com.weakly-design.ansible.json",
            r#"{"app-id":"com.weakly-design.ansible","runtime":"org.freedesktop.Platform"}"#,
        );
        write(
            root,
            "packaging/windows/ansible.wxs",
            r#"<Wix><Package Name="$(var.ProductName)" Id="*" Guid="00112233-4455-6677-8899-aabbccddeeff" /></Wix>"#,
        );
    }

    #[test]
    fn ansible_fixture_exercises_every_artifact_field() {
        let dir = tempdir().expect("temp fixture");
        ansible_fixture(dir.path());
        let report = doctor::verify(dir.path()).expect("verify fixture");
        assert!(!report.has_failures(), "{:#?}", report.checks);
        for field in [
            "Cargo.toml:bundle.identifier",
            "Cargo.toml:bundle.name",
            "Cargo.toml:bundle.icon",
            "Info.plist:CFBundleIdentifier",
            "Info.plist:CFBundleName",
            "Info.plist:NSMicrophoneUsageDescription",
            "entitlements.plist:com.apple.security.device.audio-input",
            "packaging/ansible.desktop:Name",
            "packaging/ansible.desktop:Exec",
            "packaging/ansible.desktop:Icon",
            "packaging/ansible.desktop:StartupWMClass",
            "packaging/windows/AppxManifest.xml:Identity.Name",
            "packaging/linux/com.weakly-design.ansible.json:id",
            "packaging/windows/ansible.wxs:GUID",
            "packaging/windows/ansible.wxs:Name",
        ] {
            assert!(
                report.checks.iter().any(|check| check.field == field),
                "missing {field}: {:#?}",
                report.checks
            );
        }
    }

    #[test]
    fn declared_entitlement_absent_from_file_is_missing() {
        // A present-but-wrong-keyed entitlements file must not satisfy a declared
        // entitlement; the required key is reported Missing.
        let dir = tempdir().expect("temp fixture");
        ansible_fixture(dir.path());
        write(
            dir.path(),
            "entitlements.plist",
            "<plist><dict><key>com.apple.security.network.client</key><true/></dict></plist>",
        );
        let report = doctor::verify(dir.path()).expect("verify fixture");
        assert!(report.has_failures());
        assert!(report.checks.iter().any(|check| {
            check.field == "entitlements.plist:com.apple.security.device.audio-input"
                && check.status == doctor::Status::Missing
        }));
    }

    #[test]
    fn reports_mismatch_and_missing_fields() {
        let dir = tempdir().expect("temp fixture");
        ansible_fixture(dir.path());
        write(
            dir.path(),
            "packaging/ansible.desktop",
            "[Desktop Entry]\nName=Wrong\nExec=wrong\nStartupWMClass=ansible\n",
        );
        let report = doctor::verify(dir.path()).expect("verify fixture");
        assert!(report.has_failures());
        assert!(report.checks.iter().any(
            |check| check.field.ends_with(":Name") && check.status == doctor::Status::Mismatch
        ));
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.field.ends_with(":Icon")
                    && check.status == doctor::Status::Missing)
        );
    }

    #[test]
    fn usage_description_checks_presence_not_wording() {
        let dir = tempdir().expect("temp fixture");
        ansible_fixture(dir.path());
        write(
            dir.path(),
            "Info.plist",
            r#"<plist><dict><key>CFBundleIdentifier</key><string>com.weakly-design.ansible</string><key>CFBundleName</key><string>Ansible</string><key>NSMicrophoneUsageDescription</key><string>Different nonempty wording</string></dict></plist>"#,
        );
        let report = doctor::verify(dir.path()).expect("verify fixture");
        assert!(!report.has_failures(), "{:#?}", report.checks);
    }

    #[test]
    fn msix_identity_name_requires_exact_attribute() {
        let dir = tempdir().expect("temp fixture");
        ansible_fixture(dir.path());
        write(
            dir.path(),
            "packaging/windows/AppxManifest.xml",
            r#"<Package><Identity PublisherName="com.weakly-design.ansible" /></Package>"#,
        );
        let report = doctor::verify(dir.path()).expect("verify fixture");
        assert!(report.checks.iter().any(|check| {
            check.field.ends_with("Identity.Name") && check.status == doctor::Status::Missing
        }));
    }

    #[test]
    fn accepts_flatpak_id_and_ignores_unrelated_json() {
        let dir = tempdir().expect("temp fixture");
        fs::write(dir.path().join("Cargo.toml"), manifest("com.example.app"))
            .expect("write fixture");
        write(
            dir.path(),
            "packaging/linux/manifest.json",
            r#"{"note":"id","metadata":{"id":"wrong.nested.id"},"id":"com.example.app","runtime":"org.freedesktop.Platform"}"#,
        );
        write(dir.path(), "package.json", r#"{"id":"not-an-app-id"}"#);
        let report = doctor::verify(dir.path()).expect("verify fixture");
        assert!(!report.has_failures(), "{:#?}", report.checks);
        assert_eq!(
            report
                .checks
                .iter()
                .filter(|check| check.field.ends_with(":id"))
                .count(),
            1
        );
    }

    #[test]
    fn reports_missing_flatpak_id() {
        let dir = tempdir().expect("temp fixture");
        fs::write(dir.path().join("Cargo.toml"), manifest("com.example.app"))
            .expect("write fixture");
        write(
            dir.path(),
            "packaging/flatpak/manifest.json",
            r#"{"runtime":"org.freedesktop.Platform"}"#,
        );
        let report = doctor::verify(dir.path()).expect("verify fixture");
        assert!(report.checks.iter().any(|check| {
            check.field.ends_with(":id") && check.status == doctor::Status::Missing
        }));
    }

    #[test]
    fn clean_and_mismatch_exit_codes_follow_contract() {
        let dir = tempdir().expect("temp fixture");
        fs::write(dir.path().join("Cargo.toml"), manifest("com.example.app"))
            .expect("write fixture");
        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(
            run([dir.path().display().to_string()], &mut output, &mut errors),
            0
        );

        fs::write(dir.path().join("Cargo.toml"), manifest("com.example.wrong"))
            .expect("write mismatch");
        output.clear();
        assert_eq!(
            run([dir.path().display().to_string()], &mut output, &mut errors),
            1
        );
        assert!(
            String::from_utf8(output)
                .expect("UTF-8 output")
                .contains("MISMATCH")
        );
    }

    #[test]
    fn usage_errors_exit_two_and_json_is_machine_shaped() {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(run(["--bad".to_owned()], &mut output, &mut errors), 2);
        assert!(
            String::from_utf8(errors)
                .expect("UTF-8 error")
                .contains("unknown option")
        );

        let report = doctor::Report {
            checks: vec![doctor::Check {
                field: "a\"b".to_owned(),
                manifest_value: "x".to_owned(),
                artifact_value: "y".to_owned(),
                status: doctor::Status::Ok,
            }],
        };
        write_json(&mut output, &report).expect("write JSON");
        assert!(
            String::from_utf8(output)
                .expect("UTF-8 JSON")
                .contains(r#""field":"a\"b""#)
        );
    }
}
