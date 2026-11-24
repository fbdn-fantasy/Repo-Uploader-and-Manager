use super::{Format, Result, Version, PRODUCT};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const NAME: &str = "repo-uploader-and-manager";
const LAYOUT_ERROR: &str = "Cannot identify this Linux installation. Run the app from its installed package or extracted release folder.";
const WRITE_ERROR: &str = "Could not update the extracted app. Check that its folder is writable and has enough free space.";

pub(super) enum Installation {
    Deb,
    Rpm,
    Tar(PathBuf),
}

impl Installation {
    pub(super) fn detect() -> Result<Self> {
        let exe = std::env::current_exe().map_err(|_| LAYOUT_ERROR)?;
        if exe.parent() == Some(Path::new("/usr/bin")) {
            let owned_by_rpm = Command::new("/usr/bin/rpm")
                .args(["-qf", "--queryformat", "%{NAME}"])
                .arg(&exe)
                .output()
                .is_ok_and(|out| out.status.success() && out.stdout == NAME.as_bytes());
            if owned_by_rpm {
                return Ok(Self::Rpm);
            }
            let owned_by_deb = Command::new("/usr/bin/dpkg-query")
                .arg("--search")
                .arg(&exe)
                .output()
                .is_ok_and(|out| {
                    out.status.success()
                        && String::from_utf8_lossy(&out.stdout).lines().any(|line| {
                            line.split_once(": ")
                                .is_some_and(|(package, _)| package.split(':').next() == Some(NAME))
                        })
                });
            if owned_by_deb {
                return Ok(Self::Deb);
            }
            // A TAR extracted over /usr needs a system install on the next update.
            if Path::new("/usr/bin/apt-get").is_file() {
                return Ok(Self::Deb);
            }
            if Path::new("/usr/bin/rpm").is_file() {
                return Ok(Self::Rpm);
            }
            return Err(
                "This system installation requires a supported package manager (APT or RPM)."
                    .into(),
            );
        }
        let suffix =
            PathBuf::from("usr/bin").join(format!("{PRODUCT}-V{}", env!("CARGO_PKG_VERSION")));
        if !exe.ends_with(&suffix) {
            return Err(LAYOUT_ERROR.into());
        }
        let root = exe.ancestors().nth(3).ok_or(LAYOUT_ERROR)?;
        if !root.join("usr/bin").join(NAME).is_file() {
            return Err(LAYOUT_ERROR.into());
        }
        Ok(Self::Tar(root.to_owned()))
    }

    pub(super) fn format(&self) -> Format {
        match self {
            Self::Deb => Format::Deb,
            Self::Rpm => Format::Rpm,
            Self::Tar(_) => Format::Tar,
        }
    }

    pub(super) fn apply(&self, file: &Path, current: &Version, next: &Version) -> Result<()> {
        let mut command = Command::new("/usr/bin/pkexec");
        match self {
            Self::Deb => command.args(["/usr/bin/apt-get", "-y", "install"]),
            Self::Rpm if Path::new("/usr/bin/dnf").is_file() => {
                command.args(["/usr/bin/dnf", "-y", "install"])
            }
            Self::Rpm => command.args(["/usr/bin/rpm", "--upgrade"]),
            Self::Tar(root) => return unpack_update(root, file, current, next),
        };
        let status = command.arg(file).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status()
            .map_err(|_| "Could not start the package manager. Check that polkit and the system package manager are installed.")?;
        match status.code() {
            Some(0) => Ok(()),
            Some(126) => Err("Update installation was cancelled.".into()),
            Some(127) => Err("Could not authorize the update. Check that your desktop authentication agent is running.".into()),
            _ => Err("The package manager could not install the update. Check available space and package dependencies, then retry.".into()),
        }
    }
}

// The published TAR mirrors the DEB layout. Replace only the application's own
// files, leaving other applications or user files under the extraction root alone.
fn payload_paths(version: &Version) -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from(format!("usr/bin/{PRODUCT}-V{version}")),
        "usr/share/applications/io.github.repo-uploader-and-manager.desktop.desktop".into(),
    ];
    for license in ["LICENSE", "NOTICE", "THIRD-PARTY.txt"] {
        paths.push(Path::new("usr/lib/Repo Uploader and Manager").join(license));
    }
    for size in [32, 128, 256] {
        paths.push(format!("usr/share/icons/hicolor/{size}x{size}/apps/{NAME}.png").into());
    }
    // Switch the stable launcher after its new executable and resources are in place.
    paths.push(format!("usr/bin/{NAME}").into());
    paths
}

fn unpack_update(root: &Path, archive: &Path, current: &Version, next: &Version) -> Result<()> {
    let stage = tempfile::Builder::new()
        .prefix(".repo-manager-update-")
        .tempdir_in(root)
        .map_err(|_| WRITE_ERROR)?;
    let status = Command::new("/usr/bin/tar")
        .arg("--extract")
        .arg("--gzip")
        .arg("--file")
        .arg(archive)
        .arg("--directory")
        .arg(stage.path())
        .args(["--no-same-owner", "--no-same-permissions"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "Could not unpack the update. Check that GNU tar is installed.")?;
    if !status.success() {
        return Err("The update archive could not be unpacked.".into());
    }
    let paths = payload_paths(next);
    let backups = stage.path().join(".backups");
    fs::create_dir(&backups).map_err(|_| WRITE_ERROR)?;
    for (index, relative) in paths.iter().enumerate() {
        let source = stage.path().join(relative);
        if !source.symlink_metadata().is_ok_and(|m| m.is_file())
            || source.canonicalize().ok().as_ref() != Some(&source)
        {
            return Err("The update archive is missing an expected application file.".into());
        }
        let dest = root.join(relative);
        let parent = dest.parent().ok_or(LAYOUT_ERROR)?;
        if parent.canonicalize().ok().as_deref() != Some(parent) {
            return Err(LAYOUT_ERROR.into());
        }
        match dest.symlink_metadata() {
            Ok(meta) if meta.is_file() => fs::hard_link(&dest, backups.join(index.to_string())).map_err(|_| WRITE_ERROR)?,
            Ok(_) => return Err("An application file was replaced by a directory or symbolic link. Restore the TAR installation and retry.".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(WRITE_ERROR.into()),
        }
    }
    for (index, relative) in paths.iter().enumerate() {
        if fs::rename(stage.path().join(relative), root.join(relative)).is_err() {
            let mut restored = true;
            for previous in (0..index).rev() {
                let backup = backups.join(previous.to_string());
                let dest = root.join(&paths[previous]);
                let result = if backup.exists() {
                    fs::rename(backup, dest)
                } else {
                    fs::remove_file(dest)
                };
                restored &= result.is_ok();
            }
            if !restored {
                let _ = stage.keep();
                return Err("The update could not finish restoring the previous files. Re-extract the release archive before restarting.".into());
            }
            return Err(WRITE_ERROR.into());
        }
    }
    // Bookmarks that point straight to the previous version will also launch the
    // update. The running Linux process can finish using its old executable inode.
    let alias = stage.path().join("previous-launcher");
    let old_binary = root.join(format!("usr/bin/{PRODUCT}-V{current}"));
    if std::os::unix::fs::symlink(NAME, &alias).is_ok() {
        let _ = fs::rename(alias, old_binary);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tree(root: &Path, version: &Version) {
        for relative in payload_paths(version) {
            let file = root.join(relative);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let bytes = if file.file_name().unwrap() == NAME {
                format!("#!/bin/sh\nexec \"$(dirname \"$0\")/{PRODUCT}-V{version}\"\n")
            } else {
                format!("#!/bin/sh\necho {version}\n")
            };
            fs::write(&file, bytes).unwrap();
            fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn tar_update_installs_executable_resources_and_keeps_unrelated_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("installed");
        let payload = temp.path().join("payload");
        let current = Version::new(1, 0, 0);
        let next = Version::new(1, 0, 1);
        tree(&root, &current);
        tree(&payload, &next);
        fs::write(root.join("usr/bin/unrelated"), "keep").unwrap();
        let tar = temp.path().join("test.tar.gz");
        let make_tar = || {
            assert!(Command::new("/usr/bin/tar")
                .arg("-czf")
                .arg(&tar)
                .arg("-C")
                .arg(&payload)
                .arg("usr")
                .status()
                .unwrap()
                .success())
        };
        make_tar();
        unpack_update(&root, &tar, &current, &next).unwrap();
        for name in [
            NAME.to_owned(),
            format!("{PRODUCT}-V{current}"),
            format!("{PRODUCT}-V{next}"),
        ] {
            let result = Command::new(root.join("usr/bin").join(name))
                .output()
                .unwrap();
            assert!(result.status.success());
            assert_eq!(result.stdout, b"1.0.1\n");
        }
        assert!(
            fs::read_to_string(root.join("usr/lib/Repo Uploader and Manager/NOTICE"))
                .unwrap()
                .contains("1.0.1")
        );
        assert_eq!(
            fs::read_to_string(root.join("usr/bin/unrelated")).unwrap(),
            "keep"
        );
        // Reject an incomplete release before performing any replacements.
        fs::remove_file(payload.join("usr/lib/Repo Uploader and Manager/NOTICE")).unwrap();
        fs::write(
            payload.join("usr/lib/Repo Uploader and Manager/LICENSE"),
            "broken release",
        )
        .unwrap();
        make_tar();
        assert!(unpack_update(&root, &tar, &current, &next).is_err());
        assert!(
            fs::read_to_string(root.join("usr/lib/Repo Uploader and Manager/LICENSE"))
                .unwrap()
                .contains("1.0.1")
        );
    }
}
