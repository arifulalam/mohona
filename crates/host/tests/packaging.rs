//! The installer's files agree with each other: what the bundle config ships is what the build script
//! copies, the install hooks call commands that exist, and every part has the same version.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn text(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn the_installer_ships_what_the_build_script_copies_and_the_hooks_call_real_commands() {
    let bundle: serde_json::Value =
        serde_json::from_str(&text("app/src-tauri/tauri.bundle.conf.json")).unwrap();
    let resources = bundle["bundle"]["resources"].as_object().unwrap();
    let script = text("scripts/build-windows.ps1");
    // The browser extension travels as a folder; the script builds and copies both browsers' folders.
    assert_eq!(resources["resources/extension/"], "extension/");
    assert!(script.contains("dist\\chromium") && script.contains("dist\\firefox"));
    for (from, to) in resources.iter().filter(|(from, _)| from.ends_with(".exe")) {
        let to = to.as_str().unwrap();
        assert!(
            from.starts_with("resources/") && from.ends_with(".exe"),
            "{from}"
        );
        // The program is installed next to the app, where the app looks for it.
        assert_eq!(from.trim_start_matches("resources/"), to);
        assert!(
            script.contains(&format!("target\\release\\{to}")),
            "{to} is not copied by the script"
        );
    }
    let names: Vec<&str> = resources.values().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"extension/"));
    assert!(names.contains(&"mohona.exe") && names.contains(&"mohona-host.exe"));
    assert_eq!(
        bundle["bundle"]["windows"]["nsis"]["installMode"],
        "currentUser"
    );
    assert_eq!(
        bundle["bundle"]["windows"]["nsis"]["installerHooks"],
        "./installer-hooks.nsh"
    );

    let hooks = text("app/src-tauri/installer-hooks.nsh");
    for macro_name in [
        "NSIS_HOOK_PREINSTALL",
        "NSIS_HOOK_POSTINSTALL",
        "NSIS_HOOK_PREUNINSTALL",
    ] {
        assert!(
            hooks.contains(&format!("!macro {macro_name}")),
            "{macro_name}"
        );
    }
    // The host commands the hooks use are the ones the host offers.
    let host = text("crates/host/src/main.rs");
    assert!(hooks.contains("mohona-host.exe\" install --browser all"));
    assert!(hooks.contains("mohona-host.exe\" uninstall --browser all"));
    assert!(host.contains("\"install\"") && host.contains("\"uninstall\""));
    assert!(host.contains("\"all\""));
    // The built programs are not kept in the source.
    assert!(text(".gitignore").contains("app/src-tauri/resources/*.exe"));
}

#[test]
fn every_part_has_the_same_version() {
    let version = |text: &str, key: &str| -> String {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key))
            .and_then(|r| r.split('"').nth(1))
            .unwrap_or_else(|| panic!("no {key}"))
            .to_string()
    };
    let workspace = version(&text("Cargo.toml"), "version = ");
    let tauri: serde_json::Value =
        serde_json::from_str(&text("app/src-tauri/tauri.conf.json")).unwrap();
    let app: serde_json::Value = serde_json::from_str(&text("app/package.json")).unwrap();
    let extension: serde_json::Value =
        serde_json::from_str(&text("extension/package.json")).unwrap();
    assert_eq!(tauri["version"], workspace.as_str());
    assert_eq!(app["version"], workspace.as_str());
    assert_eq!(extension["version"], workspace.as_str());
    let shell = text("app/src-tauri/Cargo.toml");
    assert_eq!(version(&shell, "version = "), workspace);
}
