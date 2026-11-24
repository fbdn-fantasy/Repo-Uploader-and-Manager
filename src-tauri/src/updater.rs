use crate::storage::Result;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::time::Duration;
use tauri::Emitter;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(test)]
mod tests;

const RELEASES: &str =
    "https://api.github.com/repos/fbdn-fantasy/Repo-Uploader-and-Manager/releases";
const PRODUCT: &str = "Repo-Uploader-and-Manager";
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const NETWORK_ERROR: &str = "Could not reach GitHub Releases. Check your connection and try again.";
const DISK_ERROR: &str = "Could not save the update. Check available disk space and try again.";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    id: u64,
    name: String,
    size: u64,
    state: String,
    digest: Option<String>,
}

#[derive(Clone, Copy)]
enum Format {
    #[cfg(any(windows, test))]
    Nsis,
    #[cfg(any(target_os = "linux", test))]
    Deb,
    #[cfg(any(target_os = "linux", test))]
    Rpm,
    #[cfg(any(target_os = "linux", test))]
    Tar,
}

impl Format {
    fn filename(self, version: &Version) -> String {
        match self {
            #[cfg(any(windows, test))]
            Self::Nsis => format!("{PRODUCT}-V{version}-Setup.exe"),
            #[cfg(any(target_os = "linux", test))]
            Self::Deb => format!("{PRODUCT}-V{version}.deb"),
            #[cfg(any(target_os = "linux", test))]
            Self::Rpm => format!("repo-uploader-and-manager-{version}-1.x86_64.rpm"),
            #[cfg(any(target_os = "linux", test))]
            Self::Tar => format!("{PRODUCT}-V{version}.tar.gz"),
        }
    }
}

impl Release {
    fn stable_version(&self) -> Option<Version> {
        if self.draft || self.prerelease {
            return None;
        }
        let tag = self
            .tag_name
            .strip_prefix(['v', 'V'])
            .unwrap_or(&self.tag_name);
        let version = Version::parse(tag).ok()?;
        (version.pre.is_empty() && version.build.is_empty()).then_some(version)
    }

    fn asset(&self, version: &Version, format: Format) -> Result<&Asset> {
        let filename = format.filename(version);
        let mut matches = self
            .assets
            .iter()
            .filter(|asset| asset.name == filename && asset.state == "uploaded");
        let asset = matches.next().ok_or_else(|| {
            format!("V{version} is available, but its {filename} installer has not been published.")
        })?;
        if matches.next().is_some() || asset.size == 0 || asset.size > MAX_BYTES || asset.id == 0 {
            return Err("The release contains invalid update asset details.".into());
        }
        asset.checksum()?;
        Ok(asset)
    }
}

impl Asset {
    fn checksum(&self) -> Result<&str> {
        self.digest
            .as_deref()
            .and_then(|d| d.strip_prefix("sha256:"))
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| "GitHub has not provided a SHA-256 checksum for this installer.".into())
    }
}

fn response(
    agent: &ureq::Agent,
    url: &str,
    accept: &str,
) -> Result<ureq::http::Response<ureq::Body>> {
    match agent
        .get(url)
        .header("Accept", accept)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
    {
        Ok(response) if response.status() == 200 => Ok(response),
        Ok(response) => Err(match response.status().as_u16() {
            403 | 429 => "GitHub limited this update request. Try again later.",
            404 => "The public GitHub release or installer is unavailable.",
            _ => "GitHub could not provide the requested update. Try again later.",
        }
        .into()),
        Err(_) => Err(NETWORK_ERROR.into()),
    }
}

fn newest(agent: &ureq::Agent, endpoint: &str) -> Result<Option<(Version, Release)>> {
    let mut best: Option<(Version, Release)> = None;
    for page in 1.. {
        let url = format!("{endpoint}?per_page=100&page={page}");
        let mut response = response(agent, &url, "application/vnd.github+json")?;
        let releases: Vec<Release> = response
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_json()
            .map_err(|_| "Could not read GitHub's release information.")?;
        let last_page = releases.len() < 100;
        for release in releases {
            if let Some(version) = release.stable_version() {
                if best
                    .as_ref()
                    .is_none_or(|(previous, _)| &version > previous)
                {
                    best = Some((version, release));
                }
            }
        }
        if last_page {
            break;
        }
    }
    Ok(best)
}

fn verified_copy(mut body: impl Read, mut output: impl Write, asset: &Asset) -> Result<()> {
    let expected = asset.checksum()?;
    let mut hash = Sha256::new();
    let mut received = 0u64;
    let mut chunk = [0u8; 65536];
    loop {
        let n = body.read(&mut chunk).map_err(|_| NETWORK_ERROR)?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > asset.size {
            return Err("The downloaded update exceeds its published size.".into());
        }
        hash.update(&chunk[..n]);
        output.write_all(&chunk[..n]).map_err(|_| DISK_ERROR)?;
    }
    if received != asset.size || !hex::encode(hash.finalize()).eq_ignore_ascii_case(expected) {
        return Err("The downloaded update failed verification. Try checking again.".into());
    }
    Ok(())
}

#[derive(Default)]
pub struct Updater {
    installed: Option<Version>,
}

impl Updater {
    pub fn check(&mut self, app: &tauri::AppHandle) -> Result<String> {
        if let Some(version) = &self.installed {
            return Ok(format!(
                "V{version} is installed. Close and reopen the app to use it."
            ));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            .http_status_as_error(false)
            .max_redirects(5)
            .user_agent(concat!(
                "Repo-Uploader-and-Manager/",
                env!("CARGO_PKG_VERSION")
            ))
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(300)))
            .build()
            .into();
        let current = Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo version");
        let Some((version, release)) = newest(&agent, RELEASES)? else {
            return Ok("No stable release with a Vx.y.z version has been published yet.".into());
        };
        if version.cmp_precedence(&current).is_le() {
            return Ok(format!("You're up to date (V{current})."));
        }

        #[cfg(target_os = "linux")]
        let install = linux::Installation::detect()?;
        #[cfg(target_os = "linux")]
        let format = install.format();
        #[cfg(windows)]
        let format = Format::Nsis;

        let asset = release.asset(&version, format)?;
        let _ = app.emit("update-progress", format!("Downloading V{version}…"));
        let temp = tempfile::Builder::new()
            .prefix("repo-manager-update-")
            .tempdir()
            .map_err(|_| DISK_ERROR)?;
        let installer = temp.path().join(&asset.name);
        let mut download = response(
            &agent,
            &format!("{RELEASES}/assets/{}", asset.id),
            "application/octet-stream",
        )?;
        let mut file = File::create_new(&installer).map_err(|_| DISK_ERROR)?;
        verified_copy(download.body_mut().as_reader(), &mut file, asset)?;
        file.sync_all().map_err(|_| DISK_ERROR)?;
        drop(file);
        let _ = app.emit("update-progress", format!("Installing V{version}…"));
        #[cfg(target_os = "linux")]
        {
            install.apply(&installer, &current, &version)?;
            self.installed = Some(version.clone());
            Ok(format!(
                "V{version} is installed. Close and reopen the app to use it."
            ))
        }
        #[cfg(windows)]
        {
            std::process::Command::new(&installer)
                .args(["/P", "/UPDATE", "/R"])
                .spawn()
                .map_err(|_| "Windows could not start the update installer.")?;
            // Windows holds the installer open while it is running. Keep its temporary
            // file available after this process exits and NSIS restarts the application.
            let _ = temp.keep();
            app.exit(0);
            Ok(format!("The V{version} installer has started."))
        }
    }
}
