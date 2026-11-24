use crate::{
    git,
    github::{Github, Remote},
    storage::{checkout_identity, Database, Repo, Result},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct Creation {
    pub name: String,
    pub description: String,
    pub parent: PathBuf,
    pub readme: bool,
    pub license: String,
    pub gitignore: String,
}

#[derive(Deserialize)]
pub struct Metadata {
    pub name: String,
    pub description: String,
    pub private: bool,
}

pub fn validate_name(name: &str) -> Result<()> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (["COM", "LPT"].iter().any(|p| stem.starts_with(p))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if name.is_empty()
        || name.len() > 100
        || name.starts_with('.')
        || name.ends_with('.')
        || name.ends_with(".git")
        || reserved
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err("Use a repository name of 1–100 letters, numbers, periods, hyphens or underscores that is also a valid folder name.".into());
    }
    Ok(())
}

fn validate_description(value: &str) -> Result<()> {
    if value.chars().count() > 350 || value.chars().any(char::is_control) {
        return Err("Use a single-line description of up to 350 characters.".into());
    }
    Ok(())
}

fn license_text(key: &str) -> Result<Option<&'static str>> {
    macro_rules! template {
        ($name:literal) => {
            Some(include_str!(concat!(
                "../templates/licenses/",
                $name,
                ".txt"
            )))
        };
    }
    Ok(match key {
        "" => None,
        "apache-2.0" => template!("apache-2.0"),
        "mit" => template!("mit"),
        "gpl-3.0" => template!("gpl-3.0"),
        "lgpl-3.0" => template!("lgpl-3.0"),
        "agpl-3.0" => template!("agpl-3.0"),
        "bsd-3-clause" => template!("bsd-3-clause"),
        "mpl-2.0" => template!("mpl-2.0"),
        "unlicense" => template!("unlicense"),
        _ => return Err("Unknown license template.".into()),
    })
}

fn ignore_text(key: &str) -> Result<Option<&'static str>> {
    Ok(match key {
        "" => None,
        "Node" => Some(include_str!("../templates/ignores/Node")),
        "Rust" => Some(include_str!("../templates/ignores/Rust")),
        "Python" => Some(include_str!("../templates/ignores/Python")),
        _ => return Err("Unknown ignore template.".into()),
    })
}

fn reserve(parent: &Path, name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    let parent = parent
        .canonicalize()
        .map_err(|_| "Choose an existing parent directory.")?;
    let path = parent.join(name);
    fs::create_dir(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => "The destination already exists. Choose another parent or repository name.",
        _ => "The repository folder could not be created. Check the parent directory and permissions.",
    })?;
    Ok(path)
}

fn write_new(path: &Path, text: &str) -> Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|_| {
            "Could not create an initial file. Check write permissions and existing files.".into()
        })
}

fn initial_files(path: &Path, request: &Creation) -> Result<()> {
    if request.readme {
        write_new(&path.join("README.md"), &format!("# {}\n", request.name))?;
    }
    if let Some(text) = license_text(&request.license)? {
        write_new(&path.join("LICENSE"), text)?;
    }
    if let Some(text) = ignore_text(&request.gitignore)? {
        write_new(&path.join(".gitignore"), text)?;
    }
    Ok(())
}

fn local_record(path: PathBuf, account: u64) -> Result<Repo> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Select a named repository folder.")?
        .to_owned();
    Ok(Repo {
        id: Uuid::new_v4().to_string(),
        account_id: account.to_string(),
        identity: checkout_identity(&path)?,
        path,
        name,
        full_name: None,
        remote_id: None,
        first_push: false,
        previous_origin: None,
    })
}

fn associate(repo: &mut Repo, remote: &Remote) {
    repo.remote_id = Some(remote.id);
    repo.full_name = Some(remote.full_name.clone());
    repo.name = remote.name.clone();
}

#[derive(Serialize)]
pub struct Created {
    pub repo: Repo,
    pub warning: Option<String>,
}

/// Link the verified destination to the existing checkout. Persist the association
/// only once Git has an origin, so an interrupted attempt can be resumed by name.
fn attach(db: &mut Database, repo: &Repo, remote: Remote, first_push: bool) -> Result<Repo> {
    verify_local(repo)?;
    let full = &remote.full_name;
    match git::connection_origin(&repo.path)? {
        Some(current) if !current.eq_ignore_ascii_case(full) => {
            return Err("This checkout's origin points to a different repository. Select its existing destination or review the origin in Git.".into());
        }
        Some(_) => {}
        None => git::set_origin(&repo.path, full, None)?,
    }
    let mut linked = repo.clone();
    associate(&mut linked, &remote);
    linked.first_push = first_push;
    db.put(&linked)?;
    Ok(linked)
}

fn create_destination(
    db: &mut Database,
    api: &Github,
    repo: &Repo,
    name: &str,
    description: &str,
) -> Result<Repo> {
    validate_name(name)?;
    if git::connection_origin(&repo.path)?.is_some() {
        return Err("This checkout already has a GitHub origin. Choose the existing repository to connect it.".into());
    }
    let remote = api.create(db.active()?, name, description)?;
    let full = remote.full_name.clone();
    attach(db, repo, remote, true).map_err(|e| {
        format!(
            "GitHub created {full}. Choose this existing private repository when you retry. {e}"
        )
    })
}

pub fn connect(
    db: &mut Database,
    api: &Github,
    repo: &Repo,
    mode: &str,
    target: &str,
) -> Result<Repo> {
    verify_local(repo)?;
    if repo.full_name.is_some() {
        return Err(
            "This checkout already has a GitHub destination. Refresh the repository.".into(),
        );
    }
    match mode {
        "create" => create_destination(db, api, repo, target, ""),
        "existing" => {
            let remote = api.repository(db.active()?, target)?;
            if !remote.full_name.eq_ignore_ascii_case(target) || !remote.private {
                return Err(
                    "Choose an existing private GitHub repository at its current owner and name."
                        .into(),
                );
            }
            if !remote.permissions.push {
                return Err(
                    "This account needs write access to the selected private repository.".into(),
                );
            }
            attach(db, repo, remote, false)
        }
        _ => Err("Choose how to connect the GitHub repository.".into()),
    }
}

pub fn create(db: &mut Database, api: &Github, request: Creation) -> Result<Created> {
    let account = db.active()?;
    validate_description(&request.description)?;
    license_text(&request.license)?;
    ignore_text(&request.gitignore)?;
    let path = reserve(&request.parent, &request.name)?;
    let local = || -> Result<Repo> {
        git::init(&path)?;
        initial_files(&path, &request)?;
        local_record(path.clone(), account)
    };
    let repo = local().map_err(|e| format!("Local folder retained at {}. {e}", path.display()))?;
    // Save the local copy first. An interrupted remote request is never retried automatically.
    db.put(&repo)?;
    Ok(
        match create_destination(db, api, &repo, &request.name, &request.description) {
            Ok(linked) => Created {
                repo: linked,
                warning: None,
            },
            Err(error) => Created {
                repo,
                warning: Some(format!(
                    "Local repository retained at {}. {error}",
                    path.display()
                )),
            },
        },
    )
}

pub fn clone_repo(db: &mut Database, api: &Github, full: &str, parent: &Path) -> Result<Repo> {
    let account = db.active()?;
    let remote = api.repository(account, full)?;
    let token = api.token(account)?;
    let path = reserve(parent, &remote.name)?;
    git::clone_repo(&path, &remote.full_name, &token).map_err(|e| {
        format!(
            "Clone stopped. Inspect the destination at {}. {e}",
            path.display()
        )
    })?;
    let mut repo = local_record(path, account)?;
    associate(&mut repo, &remote);
    db.put(&repo)?;
    Ok(repo)
}

pub fn open_repo(db: &mut Database, api: &Github, input: &Path) -> Result<Repo> {
    let account = db.active()?;
    let path = git::root(input)?;
    if let Some(repo) = db.data.repos.iter().find(|r| r.path == path) {
        if repo.account_id != account.to_string() {
            return Err(
                "This checkout belongs to another account. Switch to its associated account."
                    .into(),
            );
        }
        verify_local(repo)?;
        return Ok(repo.clone());
    }
    let mut repo = local_record(path.clone(), account)?;
    if let Some(full) = git::origin(&path)? {
        repo.full_name = Some(full.clone());
        // Local Git remains usable while the installation or network is unavailable.
        if let Ok(remote) = api.repository(account, &full) {
            associate(&mut repo, &remote);
        }
    }
    db.put(&repo)?;
    Ok(repo)
}

pub fn relocate(db: &mut Database, repo: &mut Repo, path: &Path) -> Result<()> {
    let path = git::root(path)?;
    if checkout_identity(&path)? != repo.identity {
        return Err("Choose the original checkout at its new location. For a copy on another disk, open it as a separate repository.".into());
    }
    if db
        .data
        .repos
        .iter()
        .any(|other| other.id != repo.id && other.path == path)
    {
        return Err("This location already belongs to another saved repository.".into());
    }
    repo.path = path;
    db.put(repo)
}

pub fn reassociate(db: &mut Database, api: &Github, repo: &mut Repo, account: u64) -> Result<()> {
    let id = account.to_string();
    if !db.data.accounts.iter().any(|a| a.id == id) {
        return Err("The selected account must be signed in.".into());
    }
    if let Some(full) = &repo.full_name {
        let accessible = api.repository(account, full)?;
        if repo.remote_id.is_some_and(|old| old != accessible.id) {
            return Err(
                "The selected account cannot access the original GitHub repository.".into(),
            );
        }
        associate(repo, &accessible);
    }
    repo.account_id = id;
    db.put(repo)
}

pub fn verify_local(repo: &Repo) -> Result<()> {
    if git::root(&repo.path)? != repo.path || checkout_identity(&repo.path)? != repo.identity {
        return Err("The saved checkout was moved or replaced. Remove its entry and open the correct repository.".into());
    }
    Ok(())
}

fn verified_remote(db: &mut Database, api: &Github, repo: &mut Repo) -> Result<Remote> {
    let full = repo
        .full_name
        .as_deref()
        .ok_or("This checkout has no associated GitHub repository.")?;
    let remote = api.repository(db.active()?, full)?;
    if let Some(id) = repo.remote_id {
        if id != remote.id {
            return Err("The GitHub repository at this name has changed. Remove this entry and reopen the intended checkout.".into());
        }
    } else {
        associate(repo, &remote);
        db.put(repo)?;
    }
    Ok(remote)
}

pub fn info(db: &mut Database, api: &Github, repo: &mut Repo) -> Result<Remote> {
    verified_remote(db, api, repo)
}

fn sync_origin(db: &mut Database, repo: &mut Repo) -> Result<()> {
    let full = repo
        .full_name
        .as_deref()
        .ok_or("This checkout has no associated GitHub repository.")?;
    let current = git::origin(&repo.path)?;
    if !current
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case(full))
    {
        if let Some(old) = repo.previous_origin.as_deref() {
            git::set_origin(&repo.path, full, Some(old))?;
        } else {
            return Err("The origin no longer matches the saved GitHub repository. Check the remote before continuing.".into());
        }
    }
    if repo.previous_origin.take().is_some() {
        db.put(repo)?;
    }
    Ok(())
}

fn apply_metadata(
    db: &mut Database,
    api: &Github,
    repo: &mut Repo,
    current: &Remote,
    new: &Metadata,
    visibility: bool,
) -> Result<()> {
    validate_name(&new.name)?;
    validate_description(&new.description)?;
    let rename = (new.name != current.name).then_some(new.name.as_str());
    let description = (new.description != current.description.as_deref().unwrap_or(""))
        .then_some(new.description.as_str());
    let privacy = (visibility && new.private != current.private).then_some(new.private);
    if rename.is_none() && description.is_none() && privacy.is_none() {
        return Ok(());
    }
    if !current.permissions.admin {
        return Err("Repository administration access is required. Check the account role and the app installation permissions.".into());
    }
    let remote = api.update(
        db.active()?,
        &current.full_name,
        rename,
        description,
        privacy,
        current.id,
    )?;
    if remote.id != current.id {
        return Err("Unexpected GitHub repository identity in update response.".into());
    }
    if remote.full_name != current.full_name {
        repo.previous_origin = Some(current.full_name.clone());
    }
    associate(repo, &remote);
    db.put(repo)
        .map_err(|e| format!("GitHub updated {}. {e}", remote.full_name))?;
    if repo.previous_origin.is_some() {
        sync_origin(db, repo).map_err(|e| {
            format!(
                "Renamed on GitHub to {}. Local origin still needs attention. {e}",
                remote.full_name
            )
        })?;
    }
    Ok(())
}

pub fn update(db: &mut Database, api: &Github, repo: &mut Repo, new: Metadata) -> Result<()> {
    verify_local(repo)?;
    let current = verified_remote(db, api, repo)?;
    sync_origin(db, repo)?;
    if repo.first_push && !new.private {
        return Err("Complete the first push before making this repository public.".into());
    }
    apply_metadata(db, api, repo, &current, &new, true)
}

pub fn sync(
    db: &mut Database,
    api: &Github,
    repo: &mut Repo,
    operation: &str,
    publish: Option<Metadata>,
) -> Result<()> {
    sync_using(db, api, repo, operation, publish, git::transfer)
}

fn sync_using(
    db: &mut Database,
    api: &Github,
    repo: &mut Repo,
    operation: &str,
    publish: Option<Metadata>,
    transfer: impl FnOnce(&Path, &str, &str, &str) -> Result<()>,
) -> Result<()> {
    if !["push", "fetch", "pull"].contains(&operation) {
        return Err("Unknown synchronization operation.".into());
    }
    verify_local(repo)?;
    let account = db.active()?;
    let remote = verified_remote(db, api, repo)?;
    sync_origin(db, repo)?;
    if operation == "push" && !remote.permissions.push {
        return Err("Push requires write access to repository contents for this account and app installation.".into());
    }
    let first = operation == "push" && repo.first_push;
    let mut make_public = false;
    if first {
        let metadata =
            publish.ok_or("Confirm the repository details and visibility for the first push.")?;
        if !remote.private {
            return Err("This repository is already public on GitHub. Change it to private before the first push.".into());
        }
        apply_metadata(db, api, repo, &remote, &metadata, false)?;
        make_public = !metadata.private;
        if !verified_remote(db, api, repo)?.private {
            return Err(
                "Repository privacy changed before the push. Review its GitHub settings.".into(),
            );
        }
    } else if publish.is_some() {
        return Err("Publishing options apply only to the first push.".into());
    }
    let full = repo.full_name.clone().ok_or("Missing GitHub repository.")?;
    let token = api.token(account)?;
    transfer(&repo.path, operation, &full, &token)?;
    if first {
        repo.first_push = false;
        db.put(repo)
            .map_err(|e| format!("Push succeeded. GitHub visibility was left private. {e}"))?;
        if make_public {
            verified_remote(db,api,repo).and_then(|_| api.update(account,&full,None,None,Some(false),remote.id))
                .and_then(|result| if !result.private {Ok(())} else {Err("GitHub still reports Private.".into())})
                .map_err(|e|format!("Push verified. The change to Public could not be confirmed. Check Repository Settings. {e}"))?;
        }
    }
    Ok(())
}

fn validate_local_delete(db: &Database, repo: &Repo) -> Result<()> {
    verify_local(repo)?;
    let path = &repo.path;
    if path.parent().is_none()
        || ["HOME", "USERPROFILE"]
            .iter()
            .filter_map(std::env::var_os)
            .filter_map(|p| PathBuf::from(p).canonicalize().ok())
            .any(|home| home.starts_with(path))
        || db
            .data
            .repos
            .iter()
            .any(|r| r.id != repo.id && r.path.starts_with(path))
        || !fs::symlink_metadata(path.join(".git")).is_ok_and(|m| m.file_type().is_dir())
        || path.join(".git/worktrees").exists()
    {
        return Err("This folder contains another checkout, a home directory or shared Git metadata. Remove the list entry and manage the folder manually.".into());
    }
    Ok(())
}

pub fn delete(
    db: &mut Database,
    api: &Github,
    repo: &mut Repo,
    mode: &str,
    confirmation: &str,
) -> Result<()> {
    if confirmation != repo.full_name.as_deref().unwrap_or(&repo.name) {
        return Err("Confirmation must match the exact repository name.".into());
    }
    let (local, remote) = match mode {
        "remove" => (false, false),
        "local" => (true, false),
        "remote" => (false, true),
        "both" => (true, true),
        _ => return Err("Unknown deletion option.".into()),
    };
    // A previously removed checkout (including after a failed settings save)
    // only needs its list entry cleared. A dangling link or replacement still
    // has a directory entry and must pass the normal identity checks.
    let missing = mode == "local"
        && fs::symlink_metadata(&repo.path)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
    if local && remote {
        validate_local_delete(db, repo)?;
    }
    if remote {
        let target = verified_remote(db, api, repo)?;
        if !confirmation.eq_ignore_ascii_case(&target.full_name) || !target.permissions.admin {
            return Err("Check the current full repository name and your administration permission before deleting.".into());
        }
        api.delete(db.active()?, &target.full_name, target.id)?;
        repo.full_name = None;
        repo.previous_origin = None;
        repo.remote_id = None;
        repo.first_push = false;
        db.put(repo)
            .map_err(|e| format!("The GitHub copy was deleted. The local folder was kept. {e}"))?;
        if !local {
            return Ok(());
        }
    }
    if local && !missing {
        validate_local_delete(db, repo)
            .and_then(|_| {
                fs::remove_dir_all(&repo.path).map_err(|_| {
                    "Folder deletion did not finish. Inspect the remaining files.".to_string()
                })
            })
            .map_err(|e| {
                if remote {
                    format!("GitHub copy deleted. {e}")
                } else {
                    e
                }
            })?;
    }
    db.change(|s| s.repos.retain(|r| r.id != repo.id))
}

#[cfg(test)]
mod tests;
