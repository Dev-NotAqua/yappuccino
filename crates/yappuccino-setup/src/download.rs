//! The setup program as built, with no archive in it, downloads the newest
//! Windows ZIP from GitHub's releases instead. Windows 10 and 11 carry curl
//! and bsdtar (`tar.exe`, which reads ZIPs) in System32, so it needs no HTTP
//! or ZIP code of its own; it checks each download against the SHA-256 the
//! release lists for it.

/// Where releases are published, newest name first: the repository was
/// Yapfast before it was Yappuccino.
pub const REPOSITORIES: [&str; 2] = ["Dev-NotAqua/yappuccino", "Dev-NotAqua/yapfast"];

/// A release's Windows ZIP.
#[derive(Debug, PartialEq)]
pub struct Release {
    pub tag: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex, when GitHub lists it.
    pub sha256: Option<String>,
}

/// The newest release's Windows ZIP in GitHub's `/releases` JSON, which
/// lists the newest first. Each asset lists its size and digest before its
/// download address, so the last of each before the address is its own.
pub fn newest_windows_zip(json: &str) -> Option<Release> {
    let key = "\"browser_download_url\"";
    for (at, _) in json.match_indices(key) {
        let url = string_after(&json[at + key.len()..])?;
        if !url.ends_with("-windows-x64.zip") {
            continue;
        }
        let before = &json[..at];
        let asset = &before[before.rfind("\"name\"")?..];
        let size = number_after(&asset[asset.find("\"size\"")? + 6..])?;
        let sha256 = asset
            .find("\"digest\"")
            .and_then(|at| string_after(&asset[at + 8..]))
            .and_then(|digest| digest.strip_prefix("sha256:").map(str::to_lowercase));
        let tag = url.split("/releases/download/").nth(1)?.split('/').next()?;
        return Some(Release {
            tag: tag.to_string(),
            url: url.to_string(),
            size,
            sha256,
        });
    }
    None
}

/// The JSON string after a key's colon.
fn string_after(text: &str) -> Option<&str> {
    let rest = text
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

fn number_after(text: &str) -> Option<u64> {
    let rest = text.trim_start().strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[cfg(windows)]
pub use windows::*;

#[cfg(windows)]
mod windows {
    use super::*;
    use std::io;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    const NO_WINDOW: u32 = 0x0800_0000;

    /// Windows' own copy, never one a folder on PATH might put first.
    fn system_tool(name: &str) -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join(name)
    }

    fn curl() -> Command {
        let mut command = Command::new(system_tool("curl.exe"));
        command
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--retry",
                "2",
            ])
            .args(["--user-agent", "yappuccino-setup"])
            .stdin(Stdio::null())
            .creation_flags(NO_WINDOW);
        command
    }

    /// The newest release with a Windows ZIP, from the first repository that has one.
    pub fn find_release() -> io::Result<Release> {
        for repository in REPOSITORIES {
            let output = curl()
                .arg(format!(
                    "https://api.github.com/repos/{repository}/releases?per_page=10"
                ))
                .output()?;
            if !output.status.success() {
                continue;
            }
            if let Some(release) = newest_windows_zip(&String::from_utf8_lossy(&output.stdout)) {
                return Ok(release);
            }
        }
        Err(io::Error::other(
            "Couldn't find a Yappuccino release for Windows on GitHub. Check the internet connection and try again.",
        ))
    }

    /// Downloads the ZIP into `file`, telling `progress` the share done.
    pub fn fetch(release: &Release, file: &Path, mut progress: impl FnMut(f32)) -> io::Result<()> {
        let _ = std::fs::remove_file(file);
        let mut child = curl()
            .arg("--output")
            .arg(file)
            .arg(&release.url)
            .stderr(Stdio::piped())
            .spawn()?;
        // curl writes as it goes, so the file's size is how far it is.
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            let done = std::fs::metadata(file).map(|meta| meta.len()).unwrap_or(0);
            progress(done as f32 / release.size.max(1) as f32);
            std::thread::sleep(std::time::Duration::from_millis(150));
        };
        if !status.success() {
            let mut why = String::new();
            let _ = io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut why);
            return Err(io::Error::other(format!(
                "The download stopped: {}",
                why.trim()
            )));
        }
        if std::fs::metadata(file)?.len() != release.size {
            return Err(io::Error::other(
                "The download came out the wrong size. Try again.",
            ));
        }
        if let Some(expected) = &release.sha256 {
            if &sha256(file)? != expected {
                return Err(io::Error::other(
                    "The download doesn't match its checksum, so it wasn't installed. Try again.",
                ));
            }
        }
        Ok(())
    }

    /// certutil prints the hash on its second line.
    fn sha256(file: &Path) -> io::Result<String> {
        let output = Command::new(system_tool("certutil.exe"))
            .arg("-hashfile")
            .arg(file)
            .arg("SHA256")
            .creation_flags(NO_WINDOW)
            .output()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text
            .lines()
            .nth(1)
            .ok_or_else(|| io::Error::other("certutil gave no hash"))?;
        Ok(line
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect::<String>()
            .to_lowercase())
    }

    /// Unpacks the ZIP into `into`, and returns the folder holding its
    /// files: the ZIP's one top folder, if it has one.
    pub fn unpack(zip: &Path, into: &Path) -> io::Result<PathBuf> {
        let _ = std::fs::remove_dir_all(into);
        std::fs::create_dir_all(into)?;
        let status = Command::new(system_tool("tar.exe"))
            .arg("-xf")
            .arg(zip)
            .arg("-C")
            .arg(into)
            .creation_flags(NO_WINDOW)
            .status()?;
        if !status.success() {
            return Err(io::Error::other("Couldn't unpack the download."));
        }
        let entries: Vec<PathBuf> = std::fs::read_dir(into)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<_>>()?;
        Ok(match entries.as_slice() {
            [only] if only.is_dir() => only.clone(),
            _ => into.to_path_buf(),
        })
    }

    /// Copies every file under `from` into `to`, but README.txt, which
    /// explains the ZIP, and the old console launcher; returns the bytes
    /// copied. A release from before the rename installs as yappuccino.exe.
    pub fn copy_tree(from: &Path, to: &Path) -> io::Result<u64> {
        let mut total = 0;
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let path = entry?.path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                total += copy_tree(&path, &to.join(&name))?;
            } else if name != "README.txt" && name != "run-yapfast.cmd" {
                let name = if name == "yapfast.exe" {
                    "yappuccino.exe"
                } else {
                    name.as_str()
                };
                total += std::fs::copy(&path, to.join(name))?;
            }
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from GitHub's answer for Dev-NotAqua/yapfast, pretty-printed as it comes.
    const RELEASES: &str = r#"[
  {
    "tag_name": "v0.4.0-nightly.20261108.1",
    "assets": [
      {
        "name": "SHA256SUMS.txt",
        "size": 580,
        "digest": "sha256:40e55685477bdb856f514aebb160a1a492af59713ed7b6c29d05e62243435935",
        "browser_download_url": "https://github.com/Dev-NotAqua/yapfast/releases/download/v0.4.0-nightly.20261108.1/SHA256SUMS.txt"
      },
      {
        "name": "Yapfast-0.4.0-nightly.20261108.1-windows-x64.zip",
        "uploader": { "login": "github-actions[bot]", "id": 41898282 },
        "size": 68454478,
        "digest": "sha256:0B2169D51DDAA1922187AB146E3AF396F045A34EA164654B295DA3CD8815EAA3",
        "download_count": 3,
        "browser_download_url": "https://github.com/Dev-NotAqua/yapfast/releases/download/v0.4.0-nightly.20261108.1/Yapfast-0.4.0-nightly.20261108.1-windows-x64.zip"
      }
    ]
  },
  {
    "tag_name": "v0.3.0",
    "assets": [
      {
        "name": "Yapfast-0.3.0-windows-x64.zip",
        "size": 1,
        "browser_download_url": "https://github.com/Dev-NotAqua/yapfast/releases/download/v0.3.0/Yapfast-0.3.0-windows-x64.zip"
      }
    ]
  }
]"#;

    #[test]
    fn finds_the_newest_windows_zip_with_its_own_size_and_digest() {
        assert_eq!(
            newest_windows_zip(RELEASES),
            Some(Release {
                tag: "v0.4.0-nightly.20261108.1".into(),
                url: "https://github.com/Dev-NotAqua/yapfast/releases/download/v0.4.0-nightly.20261108.1/Yapfast-0.4.0-nightly.20261108.1-windows-x64.zip".into(),
                size: 68454478,
                sha256: Some("0b2169d51ddaa1922187ab146e3af396f045a34ea164654b295da3cd8815eaa3".into()),
            })
        );
    }

    #[test]
    fn reads_compact_json_and_an_asset_without_a_digest() {
        let compact = r#"[{"assets":[{"name":"Y-0.3.0-windows-x64.zip","size":12,"browser_download_url":"https://github.com/a/b/releases/download/v0.3.0/Y-0.3.0-windows-x64.zip"}]}]"#;
        let release = newest_windows_zip(compact).unwrap();
        assert_eq!(
            (release.tag.as_str(), release.size, release.sha256),
            ("v0.3.0", 12, None)
        );
    }

    #[test]
    fn none_without_a_windows_zip() {
        assert_eq!(newest_windows_zip("[]"), None);
    }
}
