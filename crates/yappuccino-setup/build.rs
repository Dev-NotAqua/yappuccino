//! Gives the setup program Yappuccino's icon in Explorer: compiles the app's own
//! packaging/windows/yappuccino.rc, as the app's build.rs does, with llvm-rc or
//! the Windows SDK's rc.exe, and links the result.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows/yappuccino.rc");
    println!("cargo:rerun-if-changed=../../packaging/windows/yappuccino.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let resources = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("yappuccino.res");
    let folder = "../../packaging/windows";
    let status = match Command::new("llvm-rc")
        .current_dir(folder)
        .args(["-no-preprocess", "-FO"])
        .arg(&resources)
        .arg("yappuccino.rc")
        .status()
    {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let rc = windows_sdk_rc().expect("needs llvm-rc on PATH or the Windows SDK's rc.exe");
            Command::new(rc)
                .current_dir(folder)
                .arg("/fo")
                .arg(&resources)
                .arg("yappuccino.rc")
                .status()
                .expect("runs the Windows SDK's rc.exe")
        }
        Err(error) => panic!("could not run llvm-rc: {error}"),
    };
    assert!(
        status.success(),
        "the resource compiler could not compile yappuccino.rc"
    );
    println!("cargo:rustc-link-arg-bins={}", resources.display());
}

/// The newest SDK's rc.exe, where its installer puts it.
fn windows_sdk_rc() -> Option<PathBuf> {
    let mut versions: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles"]
        .iter()
        .filter_map(|variable| env::var_os(variable))
        .filter_map(|folder| {
            std::fs::read_dir(PathBuf::from(folder).join("Windows Kits/10/bin")).ok()
        })
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    versions.sort();
    versions
        .into_iter()
        .rev()
        .map(|version| version.join("x64/rc.exe"))
        .find(|path| path.is_file())
}
