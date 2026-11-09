//! Yappuccino's Windows installer. With an archive appended by
//! `yappuccino-pack`, it installs that; as built, with none, it downloads the
//! newest release from GitHub and installs it. Run with `--uninstall`, as the
//! copy it leaves in the install folder is, it uninstalls. `--quiet` does either
//! without a window, for scripts. See docs/packaging.md.
#![cfg_attr(not(test), windows_subsystem = "windows")]

// `pack` and its helpers are yappuccino-pack's; this binary only reads.
mod download;
#[allow(dead_code)]
mod pack;
#[cfg(windows)]
mod system;

slint::slint! {
    #[edition = "1.19"]
    export { SetupWindow } from "ui/setup.slint";
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "yappuccino-setup installs on Windows only; Linux has the AppImage and the Arch package."
    );
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    use slint::ComponentHandle;
    use std::path::PathBuf;
    use std::sync::Arc;

    let args: Vec<String> = std::env::args().collect();
    let uninstall = args.iter().any(|arg| arg == "--uninstall");
    let quiet = args.iter().any(|arg| arg == "--quiet");
    let exe = std::env::current_exe().expect("knows its own path");
    let payload = if uninstall {
        None
    } else {
        pack::read(&exe).ok().flatten().map(Arc::new)
    };
    let dir: PathBuf = if uninstall {
        exe.parent().unwrap().to_path_buf()
    } else {
        system::default_dir()
    };
    let task = Task {
        uninstall,
        dir,
        exe,
        payload,
        autostart: true,
        desktop: false,
    };

    if quiet {
        std::process::exit(match task.run(|_, _| ()) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("{error}");
                1
            }
        });
    }

    let window = SetupWindow::new().expect("opens a window");
    window.set_uninstall(uninstall);
    window.set_install_dir(task.dir.to_string_lossy().as_ref().into());
    window.set_version(
        task.payload
            .as_ref()
            .map(|payload| payload.version.as_str())
            .unwrap_or("")
            .into(),
    );
    window.set_downloads(!uninstall && task.payload.is_none());
    let task = std::cell::RefCell::new(Some(task));
    let weak = window.as_weak();
    window.on_start(move || {
        let Some(mut task) = task.borrow_mut().take() else {
            return;
        };
        let window = weak.unwrap();
        task.autostart = window.get_autostart();
        task.desktop = window.get_desktop();
        window.set_stage(1);
        window.set_progress(0.0);
        let weak = weak.clone();
        // The window keeps drawing while the files go out.
        std::thread::spawn(move || {
            let shown = weak.clone();
            let result = task.run(move |progress, detail: &str| {
                let detail = detail.to_string();
                let shown = shown.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = shown.upgrade() {
                        window.set_progress(progress);
                        window.set_detail(detail.into());
                    }
                });
            });
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = weak.upgrade() {
                    match result {
                        Ok(()) => window.set_stage(2),
                        Err(error) => {
                            window.set_detail(error.to_string().into());
                            window.set_stage(3);
                        }
                    }
                }
            });
        });
    });
    let dir = window.get_install_dir().to_string();
    let weak = window.as_weak();
    window.on_launch(move || {
        let _ = system::launch(std::path::Path::new(&dir));
        let _ = weak.upgrade().map(|window| window.hide());
    });
    window.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    window.run().expect("runs its window");
}

#[cfg(windows)]
struct Task {
    uninstall: bool,
    dir: std::path::PathBuf,
    exe: std::path::PathBuf,
    payload: Option<std::sync::Arc<pack::Payload>>,
    autostart: bool,
    desktop: bool,
}

#[cfg(windows)]
impl Task {
    /// Installs or uninstalls, telling `progress` how far along, from 0 to 1, and what it's doing.
    fn run(&self, mut progress: impl FnMut(f32, &str)) -> std::io::Result<()> {
        use std::io;
        progress(0.0, "Closing Yappuccino if it's running...");
        system::close_app();
        if self.uninstall {
            // Never empty a folder that isn't Yappuccino's, such as Downloads
            // after the uninstaller was copied there.
            let named = self
                .dir
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("Yappuccino"));
            if !named && !system::app_exe(&self.dir).is_file() {
                return Err(io::Error::other(format!(
                    "{} isn't a Yappuccino install folder, so nothing was removed.",
                    self.dir.display()
                )));
            }
            progress(0.3, "Removing it from Windows...");
            system::unregister(&self.dir);
            system::shortcuts(None)?;
            progress(0.6, "Removing its files...");
            system::remove_folder(&self.dir)?;
            progress(1.0, "Done");
            return Ok(());
        }
        let Some(payload) = self.payload.as_ref() else {
            return self.download(progress);
        };
        std::fs::create_dir_all(&self.dir)?;
        let total = payload.total.max(1) as f32;
        // Files take the bar to 90%; registering the rest.
        payload
            .unpack(&self.dir, |done| {
                progress(0.9 * done as f32 / total, "Copying files...")
            })
            .map_err(|error| {
                io::Error::other(format!("Couldn't copy Yappuccino's files: {error}"))
            })?;
        progress(0.9, "Adding it to Windows...");
        let mut stub = std::fs::File::open(&self.exe)?;
        let mut uninstaller = std::fs::File::create(self.dir.join(system::UNINSTALLER))?;
        io::copy(
            &mut io::Read::take(&mut stub, payload.stub_len),
            &mut uninstaller,
        )?;
        system::register(&self.dir, &payload.version, payload.total, self.autostart)?;
        system::shortcuts(Some((&system::app_exe(&self.dir), self.desktop)))?;
        progress(1.0, "Done");
        Ok(())
    }

    /// Installs the newest release from GitHub. The download takes the bar
    /// to 80%, unpacking and copying to 92%, registering the rest.
    fn download(&self, mut progress: impl FnMut(f32, &str)) -> std::io::Result<()> {
        progress(0.01, "Looking for the newest Yappuccino...");
        let release = download::find_release()?;
        // One folder per setup process: two setups at once shared one and
        // failed on each other's download (os error 32).
        let work = std::env::temp_dir().join(format!("yappuccino-setup-{}", std::process::id()));
        std::fs::create_dir_all(&work)?;
        let zip = work.join("yappuccino-windows-x64.zip");
        let megabytes = release.size / 1_000_000;
        download::fetch(&release, &zip, |share| {
            let detail = format!(
                "Downloading {} ({} of {megabytes} MB)...",
                release.tag,
                (share * megabytes as f32) as u64
            );
            progress(0.02 + 0.78 * share.min(1.0), &detail)
        })?;
        progress(0.82, "Unpacking...");
        let unpacked = download::unpack(&zip, &work.join("unpacked"))?;
        progress(0.88, "Copying files...");
        let size = download::copy_tree(&unpacked, &self.dir)?;
        if !system::app_exe(&self.dir).is_file() {
            return Err(std::io::Error::other(
                "The download holds no Yappuccino program.",
            ));
        }
        let _ = std::fs::remove_dir_all(&work);
        progress(0.92, "Adding it to Windows...");
        // As built, with no archive, this program is its own uninstaller.
        std::fs::copy(&self.exe, self.dir.join(system::UNINSTALLER))?;
        let version = release.tag.trim_start_matches('v');
        system::register(&self.dir, version, size, self.autostart)?;
        system::shortcuts(Some((&system::app_exe(&self.dir), self.desktop)))?;
        progress(1.0, "Done");
        Ok(())
    }
}
