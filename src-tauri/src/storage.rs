use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Serialize, Deserialize)]
pub struct AccountEntry {
    pub id: String,
    pub login: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub id: String,
    pub account_id: String,
    pub path: PathBuf,
    pub name: String,
    pub full_name: Option<String>,
    pub first_push: bool,
    pub remote_id: Option<u64>,
    pub identity: String,
    #[serde(default)]
    pub previous_origin: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "sync_enabled_by_default")]
    pub auto_sync_enabled: bool,
    #[serde(default)]
    pub default_git_folder: Option<PathBuf>,
    pub accounts: Vec<AccountEntry>,
    pub active_account: Option<String>,
    pub repos: Vec<Repo>,
    pub theme: String,
}

fn sync_enabled_by_default() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_sync_enabled: true,
            default_git_folder: None,
            accounts: Vec::new(),
            active_account: None,
            repos: Vec::new(),
            theme: "system".into(),
        }
    }
}

pub struct Database {
    file: PathBuf,
    pub data: Settings,
}

impl Database {
    pub fn load(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir).map_err(|_| "Cannot access the application settings directory.")?;
        let file = dir.join("workspace.json");
        let data = match fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
                "Application settings could not be read. The original file has been preserved."
            })?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Settings::default(),
            Err(_) => return Err("Cannot read the application settings file.".into()),
        };
        Ok(Self { file, data })
    }

    pub fn change(&mut self, edit: impl FnOnce(&mut Settings)) -> Result<()> {
        let mut next = self.data.clone();
        edit(&mut next);
        let mut temp = tempfile::NamedTempFile::new_in(self.file.parent().unwrap())
            .map_err(|_| "Cannot create the temporary settings file.")?;
        serde_json::to_writer_pretty(&mut temp, &next)
            .map_err(|_| "Cannot write application settings.")?;
        temp.flush()
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|_| "Cannot save application settings.")?;
        temp.persist(&self.file).map_err(|_| "Cannot replace application settings. Check available disk space and folder permissions.")?;
        self.data = next;
        Ok(())
    }

    pub fn set_default_git_folder(&mut self, folder: Option<PathBuf>) -> Result<()> {
        if let Some(path) = &folder {
            if !path.is_absolute() || !path.is_dir() {
                return Err("Choose an existing directory for the default Git folder.".into());
            }
        }
        self.change(|settings| settings.default_git_folder = folder)
    }

    pub fn active(&self) -> Result<u64> {
        self.data
            .active_account
            .as_deref()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| "Select a signed-in GitHub account.".into())
    }

    pub fn repo(&self, id: &str) -> Result<Repo> {
        let active = self.active()?.to_string();
        self.data
            .repos
            .iter()
            .find(|r| r.id == id && r.account_id == active)
            .cloned()
            .ok_or_else(|| "The selected repository does not belong to the active account.".into())
    }

    pub fn put(&mut self, repo: &Repo) -> Result<()> {
        self.change(|s| {
            if let Some(saved) = s.repos.iter_mut().find(|r| r.id == repo.id) {
                *saved = repo.clone();
            } else {
                s.repos.push(repo.clone());
            }
        })
    }
}

#[cfg(unix)]
fn legacy_file_id(path: &Path) -> std::io::Result<String> {
    use std::os::unix::fs::MetadataExt;
    let m = fs::metadata(path)?;
    Ok(format!("{}-{}", m.dev(), m.ino()))
}

#[cfg(target_os = "linux")]
pub fn file_id(path: &Path) -> std::io::Result<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(path)?;
    // Btrfs assigns different device numbers to a subvolume across mounts.
    // Its statvfs filesystem ID incorporates the filesystem and subvolume.
    let fsid = rustix::fs::statvfs(path)?.f_fsid;
    if fsid == 0 {
        return legacy_file_id(path);
    }
    let birth = meta
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|t| t.as_nanos().to_string())
        .unwrap_or_default();
    Ok(format!("fs:{fsid:x}-{}-{birth}", meta.ino()))
}

#[cfg(all(unix, not(target_os = "linux")))]
pub fn file_id(path: &Path) -> std::io::Result<String> {
    legacy_file_id(path)
}

#[cfg(windows)]
pub fn file_id(path: &Path) -> std::io::Result<String> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    Ok(format!(
        "{}-{}-{}",
        info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
    ))
}

pub fn checkout_identity(path: &Path) -> Result<String> {
    let error = |_| "Cannot verify the local checkout. Check the folder and its Git metadata.";
    Ok(format!(
        "{}/{}",
        file_id(path).map_err(error)?,
        file_id(&path.join(".git")).map_err(error)?
    ))
}

pub fn matches_checkout(saved: &str, path: &Path, current: &str) -> bool {
    if saved == current {
        return true;
    }
    // V1.0.0 saved just the device and inode pair on Unix. Accept an exact
    // match while that mount exists, never an inode match across devices.
    #[cfg(target_os = "linux")]
    if !saved.contains("fs:") {
        if let (Ok(root), Ok(git)) = (legacy_file_id(path), legacy_file_id(&path.join(".git"))) {
            return saved == format!("{root}/{git}");
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = path;
    false
}
