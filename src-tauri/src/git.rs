//! System Git is the authority for worktree, index and reference state.
use crate::storage::AccountEntry;
use serde::Serialize;
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

mod network;
mod review;
mod watcher;
pub use network::{clone_repo, transfer};
pub use review::{commit, inspect_commit};
pub use watcher::{watch, Watcher};

type Result<T> = std::result::Result<T, String>;

#[derive(Serialize)]
pub struct Change {
    pub path: String,
    pub original: Option<String>,
    pub index: String,
    pub worktree: String,
}

#[derive(Serialize)]
pub struct Commit {
    pub hash: String,
    pub title: String,
    pub author: String,
    pub date: String,
}

#[derive(Serialize)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub branch: Option<String>,
    pub changes: Vec<Change>,
    pub history: Vec<Commit>,
    pub branches: Vec<String>,
    pub ahead: u64,
    pub identity: Identity,
}

fn command() -> Command {
    let mut cmd = Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    // Ignore inherited repository/trace/credential settings. Preserve normal user Git config.
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            cmd.env_remove(key);
        }
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("LC_ALL", "C")
        .args(["-c", "core.fsmonitor=false", "-c", "color.ui=false"]);
    cmd
}

fn execute(cmd: &mut Command, input: Option<&[u8]>) -> Result<Output> {
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "Git was not found. Install system Git and restart the application.".to_owned()
            } else {
                "The Git process could not start.".to_owned()
            }
        })?;
    if let Some(bytes) = input {
        if let Some(mut pipe) = child.stdin.take() {
            // Git reads the entire path list or message before emitting its result.
            if pipe.write_all(bytes).is_err() {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Git could not read the requested operation.".into());
            }
        }
    }
    child
        .wait_with_output()
        .map_err(|_| "Git was interrupted.".into())
}

fn checked(output: Output) -> Result<Vec<u8>> {
    if output.status.success() {
        return Ok(output.stdout);
    }
    let reason = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    let message = if reason.contains("non-fast-forward")
        || reason.contains("fetch first")
        || reason.contains("not possible to fast-forward")
    {
        "The branch has diverged. Fetch and reconcile the histories with Git before retrying."
    } else if reason.contains("authentication")
        || reason.contains("403")
        || reason.contains("repository not found")
        || reason.contains("could not read username")
    {
        "GitHub denied access. Check the selected account, installation access and Contents permission, or reauthorize."
    } else if reason.contains("gh001")
        || reason.contains("large files")
        || reason.contains("100.00 mb")
    {
        "GitHub rejected a large file. Remove oversized blobs from the commits being pushed before retrying."
    } else if reason.contains("pre-receive hook")
        || reason.contains("protected branch")
        || reason.contains("gh013")
    {
        "GitHub rejected the update under its repository or branch rules."
    } else if reason.contains("could not resolve")
        || reason.contains("failed to connect")
        || reason.contains("timed out")
        || reason.contains("ssl")
        || reason.contains("rpc failed")
    {
        "Git could not complete the network connection. Check connectivity and TLS configuration."
    } else if reason.contains("lock") {
        "Git could not acquire its lock. Finish other Git operations and retry."
    } else if reason.contains("would be overwritten")
        || reason.contains("conflict")
        || reason.contains("unmerged")
    {
        "Local changes or unresolved conflicts prevent this operation. Inspect the worktree before retrying."
    } else if reason.contains("not fully merged") {
        "The branch has unmerged commits. Merge or preserve those commits before deleting it."
    } else if reason.contains("already exists") {
        "A branch, reference or destination with that name already exists."
    } else if reason.contains("permission denied") || reason.contains("dubious ownership") {
        "Git could not access the repository. Check its ownership and filesystem permissions."
    } else if reason.contains("hook") {
        "A repository hook declined the operation. Check the repository's hook configuration."
    } else {
        "Git could not complete the operation. Check the repository state, permissions and Git configuration."
    };
    // Raw diagnostics can contain credential-bearing URLs, hook output or content.
    Err(message.into())
}

fn bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    checked(execute(command().current_dir(dir).args(args), None)?)
}

fn utf8(data: &[u8]) -> Result<&str> {
    std::str::from_utf8(data).map_err(|_| {
        "This Git operation contains a name outside UTF-8. Use system Git for this repository."
            .into()
    })
}

fn line(dir: &Path, args: &[&str]) -> Result<String> {
    Ok(utf8(&bytes(dir, args)?)?
        .trim_end_matches(['\r', '\n'])
        .to_owned())
}

fn optional_line(dir: &Path, args: &[&str], missing: &[i32]) -> Result<Option<String>> {
    let output = execute(command().current_dir(dir).args(args), None)?;
    if output
        .status
        .code()
        .is_some_and(|code| missing.contains(&code))
    {
        return Ok(None);
    }
    Ok(Some(
        utf8(&checked(output)?)?
            .trim_end_matches(['\r', '\n'])
            .to_owned(),
    ))
}

pub fn version() -> Result<String> {
    Ok(utf8(&checked(execute(command().arg("--version"), None)?)?)?
        .trim()
        .to_owned())
}

pub fn root(dir: &Path) -> Result<PathBuf> {
    let requested = dir
        .canonicalize()
        .map_err(|_| "The repository folder is missing or inaccessible.")?;
    let toplevel = line(dir, &["rev-parse", "--show-toplevel"])
        .map_err(|_| "Select the root folder of a local Git working tree.")?;
    let actual = Path::new(&toplevel)
        .canonicalize()
        .map_err(|_| "The Git working tree could not be verified.")?;
    if requested != actual {
        return Err("Select the repository root, rather than a subfolder.".into());
    }
    Ok(actual)
}

pub fn init(dir: &Path) -> Result<()> {
    bytes(dir, &["init", "--initial-branch=main", "--template="])?;
    Ok(())
}

pub fn valid_full_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
}

fn github_target(url: &str) -> Option<String> {
    let repo = [
        "https://github.com/",
        "git@github.com:",
        "ssh://git@github.com/",
    ]
    .iter()
    .find_map(|prefix| url.strip_prefix(prefix))?
    .trim_end_matches('/');
    let name = repo.strip_suffix(".git").unwrap_or(repo);
    valid_full_name(name).then(|| name.to_owned())
}

fn remote_urls(dir: &Path, key: &str) -> Result<Vec<String>> {
    let Some(value) = optional_line(dir, &["config", "--get-all", key], &[1])? else {
        return Ok(vec![]);
    };
    Ok(value.split('\n').map(str::to_owned).collect())
}

pub fn origin(dir: &Path) -> Result<Option<String>> {
    let fetch = remote_urls(dir, "remote.origin.url")?;
    if fetch.len() != 1 {
        return Ok(None);
    }
    let target = github_target(&fetch[0]);
    let push = remote_urls(dir, "remote.origin.pushurl")?;
    if push.iter().any(|url| !matches!((&target,github_target(url)),(Some(a),Some(b)) if a.eq_ignore_ascii_case(&b))) {
        return Ok(None);
    }
    Ok(target)
}

pub fn connection_origin(dir: &Path) -> Result<Option<String>> {
    let current = origin(dir)?;
    if current.is_none()
        && utf8(&bytes(dir, &["remote"])?)?
            .lines()
            .any(|name| name == "origin")
    {
        return Err("This checkout already has an origin with a different Git configuration. Review its fetch and push URLs before connecting it.".into());
    }
    Ok(current)
}

pub fn set_origin(dir: &Path, full_name: &str, old: Option<&str>) -> Result<()> {
    if !valid_full_name(full_name) {
        return Err("Invalid GitHub repository name.".into());
    }
    let url = format!("https://github.com/{full_name}.git");
    match old {
        None => {
            bytes(dir, &["remote", "add", "origin", &url])?;
        }
        Some(before) => {
            if !origin(dir)?.is_some_and(|s| s.eq_ignore_ascii_case(before)) {
                return Err(
                    "The local origin has changed. Inspect its configuration before updating."
                        .into(),
                );
            }
            // Explicit push URLs belong to the user. Replacing them also risks a partial rename.
            if !remote_urls(dir, "remote.origin.pushurl")?.is_empty() {
                return Err("GitHub was renamed. Update the explicit origin fetch and push URLs in Git, then reopen the repository.".into());
            }
            let previous = remote_urls(dir, "remote.origin.url")?;
            let existing = previous.first().ok_or("The local origin is missing.")?;
            let prefix = if existing.starts_with("ssh://") {
                "ssh://git@github.com/"
            } else if existing.starts_with("git@") {
                "git@github.com:"
            } else {
                "https://github.com/"
            };
            bytes(
                dir,
                &[
                    "remote",
                    "set-url",
                    "origin",
                    &format!("{prefix}{full_name}.git"),
                ],
            )?;
        }
    }
    Ok(())
}

fn current_branch(dir: &Path) -> Result<Option<String>> {
    optional_line(dir, &["symbolic-ref", "-q", "--short", "HEAD"], &[1])
}

fn head(dir: &Path) -> Result<Option<String>> {
    optional_line(
        dir,
        &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
        &[1],
    )
}

fn changes(dir: &Path) -> Result<Vec<Change>> {
    let data = bytes(
        dir,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--find-renames",
            "--untracked-files=all",
        ],
    )?;
    let mut fields = data.split(|b| *b == 0);
    let mut result = Vec::new();
    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        if record.len() < 4 || record[2] != b' ' {
            return Err("Git returned an unreadable status entry.".into());
        }
        let original = if record[..2].iter().any(|b| matches!(b, b'R' | b'C')) {
            Some(utf8(fields.next().ok_or("Missing original filename.")?)?.to_owned())
        } else {
            None
        };
        result.push(Change {
            path: utf8(&record[3..])?.to_owned(),
            original,
            index: (record[0] as char).to_string(),
            worktree: (record[1] as char).to_string(),
        });
    }
    Ok(result)
}

fn author(dir: &Path) -> Result<Identity> {
    let read = |key: &str| optional_line(dir, &["config", "--get", key], &[1]);
    Ok(Identity {
        name: read("user.name")?.unwrap_or_default(),
        email: read("user.email")?.unwrap_or_default(),
    })
}

impl Identity {
    pub fn with_account(mut self, account: Option<&AccountEntry>) -> Self {
        if let Some(account) = account {
            if self.name.trim().is_empty() {
                self.name = account.login.clone();
            }
            if self.email.trim().is_empty() {
                self.email = format!("{}+{}@users.noreply.github.com", account.id, account.login);
            }
        }
        self
    }
}

pub fn identity(dir: &Path, name: &str, email: &str) -> Result<()> {
    root(dir)?;
    if [name, email]
        .iter()
        .any(|x| x.trim().is_empty() || x.chars().any(char::is_control) || x.contains(['<', '>']))
        || !email.contains('@')
    {
        return Err("Enter a commit author name and email address.".into());
    }
    bytes(dir, &["config", "--local", "user.name", name.trim()])?;
    bytes(dir, &["config", "--local", "user.email", email.trim()])?;
    Ok(())
}

pub fn snapshot(dir: &Path) -> Result<Snapshot> {
    root(dir)?;
    let branch = current_branch(dir)?;
    let branches = line(
        dir,
        &["for-each-ref", "--format=%(refname:strip=2)", "refs/heads/"],
    )?
    .lines()
    .map(str::to_owned)
    .collect();
    let mut history = Vec::new();
    let mut ahead = 0;
    if head(dir)?.is_some() {
        let log = bytes(
            dir,
            &[
                "log",
                "-100",
                "-z",
                "--no-show-signature",
                "--format=%h%x00%s%x00%an%x00%aI",
            ],
        )?;
        let fields = utf8(&log)?
            .trim_end_matches('\0')
            .split('\0')
            .collect::<Vec<_>>();
        for row in fields.chunks_exact(4) {
            history.push(Commit {
                hash: row[0].into(),
                title: row[1].into(),
                author: row[2].into(),
                date: row[3].into(),
            });
        }
        let upstream = optional_line(
            dir,
            &["rev-parse", "--verify", "--quiet", "@{upstream}"],
            &[1, 128],
        )?;
        let count = match upstream {
            Some(id) => line(dir, &["rev-list", "--count", &format!("{id}..HEAD")])?,
            None => line(dir, &["rev-list", "--count", "HEAD", "--not", "--remotes"])?,
        };
        ahead = count
            .parse()
            .map_err(|_| "Git could not count outgoing commits.")?;
    }
    Ok(Snapshot {
        branch,
        changes: changes(dir)?,
        history,
        branches,
        ahead,
        identity: author(dir)?,
    })
}

fn stage_paths(dir: &Path, selected: &[String], add: bool) -> Result<()> {
    root(dir)?;
    if selected.is_empty() {
        return Err("Select changes first.".into());
    }
    let status = changes(dir)?;
    let mut input = Vec::new();
    for name in selected {
        let row = status
            .iter()
            .find(|c| &c.path == name)
            .ok_or("The selected changes have moved. Refresh and select them again.")?;
        let relevant = if add {
            row.worktree != " "
        } else {
            row.index != " " && row.index != "?"
        };
        if !relevant {
            continue;
        }
        input.extend_from_slice(name.as_bytes());
        input.push(0);
        if let Some(old) = row.original.as_ref().filter(|_| {
            if add {
                row.worktree == "R"
            } else {
                row.index == "R"
            }
        }) {
            input.extend_from_slice(old.as_bytes());
            input.push(0);
        }
    }
    if input.is_empty() {
        return Err("The selection contains no applicable changes.".into());
    }
    let mut cmd = command();
    cmd.current_dir(dir);
    if add {
        cmd.args(["add", "-A"]);
    } else if head(dir)?.is_some() {
        cmd.args(["reset", "-q", "HEAD"]);
    } else {
        cmd.args(["rm", "--cached", "--force", "--ignore-unmatch"]);
    }
    cmd.args(["--pathspec-from-file=-", "--pathspec-file-nul"]);
    checked(execute(&mut cmd, Some(&input))?)?;
    Ok(())
}

pub fn stage(dir: &Path, files: &[String]) -> Result<()> {
    stage_paths(dir, files, true)
}
pub fn unstage(dir: &Path, files: &[String]) -> Result<()> {
    stage_paths(dir, files, false)
}

fn valid_branch(dir: &Path, name: &str) -> Result<()> {
    if name.is_empty()
        || name.starts_with('-')
        || name.starts_with('@')
        || name == "HEAD"
        || name.chars().any(char::is_control)
    {
        return Err("Enter a valid branch name.".into());
    }
    bytes(dir, &["check-ref-format", &format!("refs/heads/{name}")])
        .map(|_| ())
        .map_err(|_| "Enter a valid branch name.".into())
}

pub fn branch(dir: &Path, action: &str, name: &str, new_name: &str) -> Result<()> {
    root(dir)?;
    valid_branch(dir, name)?;
    match action {
        "create" => {
            bytes(dir, &["branch", "--", name])?;
        }
        "switch" => {
            // Avoid creating an implicit tracking branch merely by typing a remote name.
            bytes(dir, &["switch", "--no-guess", "--", name])?;
        }
        "rename" => {
            valid_branch(dir, new_name)?;
            bytes(dir, &["branch", "-m", "--", name, new_name])?;
        }
        "delete" => {
            if current_branch(dir)?.as_deref() == Some(name) {
                return Err("Switch branches before deleting the active branch.".into());
            }
            if !execute(
                command().current_dir(dir).args([
                    "merge-base",
                    "--is-ancestor",
                    &format!("refs/heads/{name}"),
                    "HEAD",
                ]),
                None,
            )?
            .status
            .success()
            {
                return Err("The branch contains commits outside the active branch. Merge or preserve them before deletion.".into());
            }
            bytes(dir, &["branch", "-d", "--", name])?;
        }
        _ => return Err("Unknown branch operation.".into()),
    }
    Ok(())
}

fn git_path(dir: &Path, name: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(line(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-path", name],
    )?))
}

#[cfg(test)]
mod tests;
