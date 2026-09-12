use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/earphone_lost_search_amber_cyan.ico");

    if env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let assets_dir = manifest_dir.join("assets");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let resource_path = out_dir.join("earphone_lost_search.res");
    let rc_path = find_resource_compiler().expect(
        "Windows resource compiler (rc.exe) was not found; install the Windows SDK or set RC",
    );

    let status = Command::new(&rc_path)
        .current_dir(&assets_dir)
        .args([
            "/nologo".to_owned(),
            format!("/fo{}", resource_path.display()),
            "app.rc".to_owned(),
        ])
        .status()
        .expect("failed to execute rc.exe");
    assert!(status.success(), "rc.exe failed while compiling app.rc");

    println!("cargo:rustc-link-arg-bins={}", resource_path.display());
}

fn find_resource_compiler() -> Option<PathBuf> {
    if let Some(path) = env::var_os("RC") {
        return Some(PathBuf::from(path));
    }

    if let Some(path) = find_on_path("rc.exe") {
        return Some(path);
    }

    let kits_dir = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut candidates = Vec::new();
    for version in fs::read_dir(kits_dir).ok()?.flatten() {
        let path = version.path().join("x64").join("rc.exe");
        if path.is_file() {
            candidates.push(path);
        }
    }
    candidates.sort();
    candidates.pop()
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path) {
        let candidate = directory.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
