//! Credentialed commands run in a disposable bare repository with no user,
//! system or worktree Git configuration, templates, hooks or credential helpers.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use tempfile::TempDir;

pub(super) fn private_tempdir() -> std::io::Result<TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("repo-manager-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder.tempdir()
}

struct Transport<'a> {
    temp: TempDir,
    url: String,
    token: &'a str,
    objects: Option<PathBuf>,
}

impl<'a> Transport<'a> {
    fn new(full_name: &str, token: &'a str, local: Option<&Path>) -> Result<Self> {
        if !valid_full_name(full_name) || token.is_empty() || token.chars().any(char::is_control) {
            return Err("The GitHub repository or credentials could not be verified.".into());
        }
        let temp = private_tempdir().map_err(|_| "Could not reserve temporary Git storage.")?;
        let objects = local.map(|p| git_path(p, "objects")).transpose()?;
        let transport = Self {
            temp,
            url: format!("https://github.com/{full_name}.git"),
            token,
            objects,
        };
        let mut cmd = transport.isolated();
        cmd.args(["init", "--bare", "--initial-branch=main", "--template="]);
        if let Some(dir) = local {
            let format = line(dir, &["rev-parse", "--show-object-format"])?;
            cmd.arg(format!("--object-format={format}"));
        }
        cmd.arg(transport.repo());
        checked(execute(&mut cmd, None)?)?;
        Ok(transport)
    }

    fn repo(&self) -> PathBuf {
        self.temp.path().join("transport.git")
    }

    fn isolated(&self) -> Command {
        let mut cmd = command();
        // Empty home and an explicitly missing global config prevent URL rewrites
        // and ensure the access token is only ever sent to the verified HTTPS origin.
        cmd.current_dir(self.temp.path())
            .env("HOME", self.temp.path())
            .env("XDG_CONFIG_HOME", self.temp.path())
            .env("GIT_CONFIG_GLOBAL", self.temp.path().join("no-gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_ASKPASS")
            .env_remove("SSH_ASKPASS")
            .env_remove("SSLKEYLOGFILE")
            .args([
                "-c",
                "protocol.allow=never",
                "-c",
                "protocol.https.allow=always",
                "-c",
                "http.followRedirects=false",
                "-c",
                "credential.helper=",
                "-c",
                "http.sslVerify=true",
                "-c",
                "gc.auto=0",
                "-c",
                "maintenance.auto=false",
            ]);
        #[cfg(test)]
        if self.url.starts_with("file://") {
            cmd.args(["-c", "protocol.file.allow=always"]);
        }
        cmd
    }

    fn run(&self, args: &[&str], auth: bool, input: Option<&[u8]>) -> Result<Vec<u8>> {
        let mut cmd = self.isolated();
        cmd.arg("--git-dir").arg(self.repo());
        if let Some(objects) = &self.objects {
            cmd.env("GIT_OBJECT_DIRECTORY", objects);
        }
        if auth {
            let encoded = STANDARD.encode(format!("x-access-token:{}", self.token));
            cmd.arg(format!(
                "--config-env=http.{}.extraHeader=RUM_GIT_HTTP_HEADER",
                self.url
            ))
            .env(
                "RUM_GIT_HTTP_HEADER",
                format!("Authorization: Basic {encoded}"),
            );
        }
        checked(execute(cmd.args(args), input)?)
    }

    fn remote_head(&self, reference: &str) -> Result<String> {
        Ok(utf8(&self.run(
            &["ls-remote", "--symref", "--", &self.url, reference],
            true,
            None,
        )?)?
        .to_owned())
    }

    fn fetch(&self, background: bool) -> Result<()> {
        let mut args = Vec::new();
        if background {
            // Release the checkout reservation when an unattended HTTP transfer stalls.
            args.extend(["-c", "http.lowSpeedLimit=1", "-c", "http.lowSpeedTime=60"]);
        }
        args.extend([
            "fetch",
            "--no-recurse-submodules",
            "--no-tags",
            "--",
            &self.url,
            "+refs/heads/*:refs/heads/*",
            "refs/tags/*:refs/tags/*",
        ]);
        self.run(&args, true, None)?;
        Ok(())
    }
}

pub fn clone_repo(destination: &Path, full_name: &str, token: &str) -> Result<()> {
    if !destination.is_dir()
        || destination
            .read_dir()
            .map_err(|_| "Destination folder is inaccessible.")?
            .next()
            .is_some()
    {
        return Err("Choose an empty destination folder.".into());
    }
    let bridge = Transport::new(full_name, token, None)?;
    clone_from(destination, &bridge, &bridge.url)
}

fn clone_from(destination: &Path, bridge: &Transport<'_>, origin: &str) -> Result<()> {
    bridge.fetch(false)?;
    let remote_head = bridge.remote_head("HEAD")?;
    if let Some(name) = remote_head
        .lines()
        .find_map(|s| s.strip_prefix("ref: ")?.strip_suffix("\tHEAD"))
    {
        if let Some(short) = name.strip_prefix("refs/heads/") {
            valid_branch(bridge.temp.path(), short)?;
            bridge.run(&["symbolic-ref", "HEAD", name], false, None)?;
        }
    }
    // No credential reaches checkout, template hooks, content filters or the new repository.
    let mut clone = bridge.isolated();
    clone
        .args([
            "-c",
            "protocol.file.allow=always",
            "clone",
            "--no-local",
            "--template=",
            "--",
        ])
        .arg(bridge.repo())
        .arg(destination);
    checked(execute(&mut clone, None)?)?;
    bytes(destination, &["remote", "set-url", "origin", origin])?;
    Ok(())
}

fn target_branch(dir: &Path, local: &str) -> Result<String> {
    let key = |v: &str| format!("branch.{local}.{v}");
    let remote = optional_line(dir, &["config", "--get", &key("remote")], &[1])?;
    let merge = optional_line(dir, &["config", "--get", &key("merge")], &[1])?;
    match (remote.as_deref(),merge.as_deref()) {
        (None,None)=> Ok(local.to_owned()),
        (Some("origin"),Some(reference)) if reference.starts_with("refs/heads/") => {
            let target=&reference["refs/heads/".len()..];
            valid_branch(dir,target)?;
            Ok(target.to_owned())
        }
        _=> Err("This branch tracks a different or incomplete upstream. Configure its origin tracking branch with Git.".into())
    }
}

fn refs(dir: &Path, prefix: &str) -> Result<Vec<(String, String)>> {
    parse_refs(&bytes(
        dir,
        &[
            "for-each-ref",
            "--format=%(objectname) %(refname) %(symref)",
            prefix,
        ],
    )?)
}

fn parse_refs(data: &[u8]) -> Result<Vec<(String, String)>> {
    let mut result = Vec::new();
    for line in utf8(data)?.lines() {
        let mut fields = line.split(' ');
        let (Some(id), Some(name), Some("")) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        if !id.bytes().all(|b| b.is_ascii_hexdigit())
            || !matches!(id.len(), 40 | 64)
            || !name.starts_with("refs/")
        {
            return Err("Git returned an invalid reference.".into());
        }
        result.push((name.to_owned(), id.to_owned()));
    }
    Ok(result)
}

fn fetch_into(dir: &Path, bridge: &Transport<'_>, background: bool) -> Result<()> {
    use std::collections::BTreeMap;
    let old: BTreeMap<_, _> = refs(dir, "refs/remotes/origin/")?
        .into_iter()
        .chain(refs(dir, "refs/tags/")?)
        .collect();
    let mut seed = String::new();
    for (name, id) in &old {
        let seed_name = if let Some(short) = name.strip_prefix("refs/remotes/origin/") {
            // Seed a separate namespace for negotiation without preserving remote-deleted branches.
            format!("refs/previous/{short}")
        } else {
            format!(
                "refs/previous-tags/{}",
                name.trim_start_matches("refs/tags/")
            )
        };
        seed.push_str(&format!("update {seed_name} {id}\n"));
    }
    if !seed.is_empty() {
        bridge.run(&["update-ref", "--stdin"], false, Some(seed.as_bytes()))?;
    }
    bridge.fetch(background)?;
    let fetched = parse_refs(&bridge.run(
        &[
            "for-each-ref",
            "--format=%(objectname) %(refname) %(symref)",
            "refs/heads/",
            "refs/tags/",
        ],
        false,
        None,
    )?)?;
    let mut updates = String::new();
    for (name, id) in fetched {
        let dest = if let Some(short) = name.strip_prefix("refs/heads/") {
            format!("refs/remotes/origin/{short}")
        } else {
            name
        };
        let before = old.get(&dest);
        if before == Some(&id) {
            continue;
        }
        if dest.starts_with("refs/tags/") && before.is_some() {
            return Err("A fetched tag differs from the local tag. Resolve the tag with Git before updating references.".into());
        }
        let expected = before.cloned().unwrap_or_else(|| "0".repeat(id.len()));
        updates.push_str(&format!("update {dest} {id} {expected}\n"));
    }
    if !updates.is_empty() {
        let mut cmd = command();
        if background {
            cmd.args(["-c", "core.hooksPath=/dev/null"]);
        }
        checked(execute(
            cmd.current_dir(dir).args(["update-ref", "--stdin"]),
            Some(updates.as_bytes()),
        )?)?;
    }
    Ok(())
}

fn push(dir: &Path, local: &str, target: &str, bridge: &Transport<'_>) -> Result<()> {
    let id = head(dir)?.ok_or("Create a commit before pushing.")?;
    let reference = format!("refs/heads/{target}");
    bridge.run(
        &[
            "push",
            "--porcelain",
            "--",
            &bridge.url,
            &format!("{id}:{reference}"),
        ],
        true,
        None,
    )?;
    let listing=bridge.remote_head(&reference).map_err(|_| "Git accepted the push, but the remote branch could not be verified. Visibility has been preserved.")?;
    if !listing.lines().any(|s| s == format!("{id}\t{reference}")) {
        return Err(
            "The remote branch differs from the pushed commit. Visibility has been preserved."
                .into(),
        );
    }
    bytes(
        dir,
        &["update-ref", &format!("refs/remotes/origin/{target}"), &id],
    )
    .map_err(|_| "The push succeeded. Updating the local tracking reference failed.")?;
    bytes(
        dir,
        &[
            "branch",
            &format!("--set-upstream-to=origin/{target}"),
            "--",
            local,
        ],
    )
    .map_err(|_| "The push succeeded. Configure the upstream in the local repository.")?;
    Ok(())
}

pub fn transfer(dir: &Path, action: &str, full_name: &str, token: &str) -> Result<()> {
    root(dir)?;
    if !origin(dir)?.is_some_and(|s| s.eq_ignore_ascii_case(full_name)) {
        return Err("The configured origin differs from the selected GitHub repository. Check the repository association.".into());
    }
    let bridge = Transport::new(full_name, token, Some(dir))?;
    transfer_verified(dir, action, &bridge)
}

fn transfer_verified(dir: &Path, action: &str, bridge: &Transport<'_>) -> Result<()> {
    if action == "fetch" {
        return fetch_into(dir, bridge, false);
    }
    let local = current_branch(dir)?.ok_or("Switch to a local branch first.")?;
    let target = target_branch(dir, &local)?;
    match action {
        "push" => push(dir, &local, &target, bridge),
        "pull" => {
            if !changes(dir)?.is_empty() {
                return Err("Commit or set aside local changes before pulling.".into());
            }
            super::review::check_operation(dir)?;
            let tip = fetch_branch(dir, bridge, &target, false)?
                .ok_or("The tracking branch does not exist on GitHub.")?;
            if current_branch(dir)?.as_deref() != Some(&local) || !changes(dir)?.is_empty() {
                return Err(
                    "The local branch or worktree changed during fetch. Inspect it and retry."
                        .into(),
                );
            }
            super::review::check_operation(dir)?;
            bytes(dir, &["merge", "--ff-only", "--no-edit", "--", &tip])?;
            if !bytes(dir, &["ls-files", "-u", "-z"])?.is_empty() {
                return Err("Unresolved conflicts remain in the worktree.".into());
            }
            Ok(())
        }
        _ => Err("Unknown Git transfer operation.".into()),
    }
}

/// Only use a tip received by this fetch. Remote-deleted branches may still
/// have a stale local tracking reference.
fn fetch_branch(
    dir: &Path,
    bridge: &Transport<'_>,
    target: &str,
    background: bool,
) -> Result<Option<String>> {
    fetch_into(dir, bridge, background)?;
    let reference = format!("refs/heads/{target}");
    let data = bridge.run(
        &[
            "for-each-ref",
            "--format=%(objectname) %(refname) %(symref)",
            &reference,
        ],
        false,
        None,
    )?;
    Ok(parse_refs(&data)?
        .into_iter()
        .find(|(name, _)| name == &reference)
        .map(|(_, id)| id))
}

pub enum PullResult {
    Current,
    Updated,
    Skipped(&'static str),
}

pub fn auto_ready(dir: &Path) -> Result<Option<&'static str>> {
    if current_branch(dir)?.is_none() {
        return Ok(Some("Detached HEAD. Switch to a local branch to sync."));
    }
    if super::review::check_operation(dir).is_err() {
        return Ok(Some("A Git operation is in progress."));
    }
    Ok(None)
}

pub struct FetchedPull {
    branch: String,
    head: Option<String>,
    target: String,
    tip: Option<String>,
}

pub fn prepare_auto_pull(dir: &Path, full: &str, token: &str) -> Result<FetchedPull> {
    if !origin(dir)?.is_some_and(|s| s.eq_ignore_ascii_case(full)) {
        return Err("The configured origin differs from the saved GitHub repository.".into());
    }
    let bridge = Transport::new(full, token, Some(dir))?;
    FetchedPull::fetch(dir, &bridge)
}

impl FetchedPull {
    fn fetch(dir: &Path, bridge: &Transport<'_>) -> Result<Self> {
        let branch = current_branch(dir)?.ok_or("Switch to a local branch to sync.")?;
        let head = head(dir)?;
        let target = target_branch(dir, &branch)?;
        let tip = fetch_branch(dir, bridge, &target, true)?;
        Ok(Self {
            branch,
            head,
            target,
            tip,
        })
    }

    pub fn apply(&self, dir: &Path) -> Result<PullResult> {
        if let Some(reason) = auto_ready(dir)? {
            return Ok(PullResult::Skipped(reason));
        }
        if current_branch(dir)?.as_ref() != Some(&self.branch)
            || head(dir)? != self.head
            || target_branch(dir, &self.branch)? != self.target
        {
            return Ok(PullResult::Skipped(
                "The local branch changed during synchronization.",
            ));
        }
        let Some(tip) = &self.tip else {
            return Ok(if self.head.is_none() {
                PullResult::Current
            } else {
                PullResult::Skipped("The tracking branch does not exist on GitHub.")
            });
        };
        if let Some(head) = &self.head {
            if head == tip || ancestor(dir, tip, head)? {
                return Ok(PullResult::Current);
            }
            if !ancestor(dir, head, tip)? {
                return Ok(PullResult::Skipped(
                    "Local and GitHub history diverged. Reconcile the branch manually.",
                ));
            }
        }
        if !changes(dir)?.is_empty() {
            return Ok(PullResult::Skipped(
                "Incoming commits were fetched. Commit or set aside local changes before pulling.",
            ));
        }
        // A background pull must never invoke post-merge hooks, autostash edits,
        // update submodules or overwrite ignored files with downloaded files.
        let mut args = vec![
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "submodule.recurse=false",
        ];
        if self.head.is_some() {
            args.extend([
                "merge",
                "--ff-only",
                "--no-edit",
                "--no-autostash",
                "--no-overwrite-ignore",
                "--",
                tip,
            ]);
        } else {
            // On an unborn branch 'merge' overwrites ignored files even with
            // --no-overwrite-ignore. 'switch -c' protects them and refuses to
            // reset a branch that has gained a commit since it was inspected.
            args.extend([
                "switch",
                "--no-track",
                "--no-overwrite-ignore",
                "-c",
                &self.branch,
                tip,
            ]);
        }
        bytes(dir, &args)?;
        Ok(PullResult::Updated)
    }
}

fn ancestor(dir: &Path, first: &str, second: &str) -> Result<bool> {
    let output = execute(
        command()
            .current_dir(dir)
            .args(["merge-base", "--is-ancestor", first, second]),
        None,
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err("Git could not compare local and remote history.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{record, working_tree};
    use super::*;

    fn fixture<'a>(remote: &Path, dir: Option<&Path>) -> Transport<'a> {
        let mut bridge = Transport::new("sample/repo", "fixture", dir).unwrap();
        bridge.url = format!("file://{}", remote.to_str().unwrap().replace('\\', "/"));
        bridge
    }

    #[cfg(unix)]
    #[test]
    fn temporary_repository_storage_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = private_tempdir().unwrap();
        let mode = dir.path().metadata().unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn clone_push_fetch_pull_and_rejected_updates() {
        let remote = tempfile::tempdir().unwrap();
        bytes(
            remote.path(),
            &["init", "--bare", "--initial-branch=main", "--template="],
        )
        .unwrap();
        let first = working_tree();
        let a = first.path();
        set_origin(a, "sample/repo", None).unwrap();
        record(a, "file", "original");
        let push = || transfer_verified(a, "push", &fixture(remote.path(), Some(a)));
        push().unwrap();
        assert_eq!(snapshot(a).unwrap().ahead, 0);
        let cloned = tempfile::tempdir().unwrap();
        let b = cloned.path();
        clone_from(
            b,
            &fixture(remote.path(), None),
            "https://github.com/sample/repo.git",
        )
        .unwrap();
        assert_eq!(origin(b).unwrap().as_deref(), Some("sample/repo"));
        assert_eq!(head(a).unwrap(), head(b).unwrap());
        identity(b, "Example Author", "author@example.invalid").unwrap();
        record(a, "file", "upstream");
        push().unwrap();
        let run = |action| transfer_verified(b, action, &fixture(remote.path(), Some(b)));
        run("fetch").unwrap();
        assert_eq!(std::fs::read_to_string(b.join("file")).unwrap(), "original");
        std::fs::write(b.join("file"), "edited").unwrap();
        assert!(run("pull").is_err());
        std::fs::write(b.join("file"), "original").unwrap();
        run("pull").unwrap();
        assert_eq!(head(a).unwrap(), head(b).unwrap());
        record(a, "file", "another upstream commit");
        push().unwrap();
        record(b, "file", "diverging change");
        let competing = head(b).unwrap();
        assert!(run("push").is_err());
        assert!(run("pull").is_err());
        assert_eq!(head(b).unwrap(), competing);
        assert_eq!(
            std::fs::read_to_string(b.join("file")).unwrap(),
            "diverging change"
        );
        assert_ne!(head(a).unwrap(), head(b).unwrap());
    }

    #[test]
    fn background_pull_fast_forwards_clean_branches_and_protects_local_work() {
        use std::fs;
        let server = tempfile::tempdir().unwrap();
        bytes(
            server.path(),
            &["init", "--bare", "--initial-branch=main", "--template="],
        )
        .unwrap();
        let first = working_tree();
        let cloned = tempfile::tempdir().unwrap();
        let a = first.path();
        let b = cloned.path();
        set_origin(a, "sample/repo", None).unwrap();
        record(a, "tracked", "original");
        let push = || transfer_verified(a, "push", &fixture(server.path(), Some(a))).unwrap();
        push();
        clone_from(
            b,
            &fixture(server.path(), None),
            "https://github.com/sample/repo.git",
        )
        .unwrap();
        identity(b, "Example", "example@example.invalid").unwrap();
        let fetch = || FetchedPull::fetch(b, &fixture(server.path(), Some(b))).unwrap();
        assert!(matches!(fetch().apply(b).unwrap(), PullResult::Current));
        record(a, "tracked", "incoming");
        push();
        fs::write(b.join("tracked"), "staged local").unwrap();
        bytes(b, &["add", "tracked"]).unwrap();
        fs::write(b.join("tracked"), "unstaged local").unwrap();
        let local_before = head(b).unwrap();
        assert!(auto_ready(b).unwrap().is_none());
        let fetched = fetch();
        assert!(matches!(fetched.apply(b).unwrap(), PullResult::Skipped(_)));
        assert_eq!(head(b).unwrap(), local_before);
        assert_eq!(
            line(b, &["rev-parse", "refs/remotes/origin/main"]).unwrap(),
            head(a).unwrap().unwrap()
        );
        assert_eq!(line(b, &["show", ":tracked"]).unwrap(), "staged local");
        assert_eq!(
            fs::read_to_string(b.join("tracked")).unwrap(),
            "unstaged local"
        );
        bytes(
            b,
            &[
                "restore",
                "--source=HEAD",
                "--staged",
                "--worktree",
                "tracked",
            ],
        )
        .unwrap();
        assert!(matches!(fetched.apply(b).unwrap(), PullResult::Updated));
        assert_eq!(fs::read_to_string(b.join("tracked")).unwrap(), "incoming");
        // An ignored local file may have been newly tracked on GitHub.
        fs::create_dir_all(b.join(".git/info")).unwrap();
        fs::write(b.join(".git/info/exclude"), "ignored\n").unwrap();
        fs::write(b.join("ignored"), "private local data").unwrap();
        record(a, "ignored", "upstream");
        push();
        let previous = head(b).unwrap();
        assert!(auto_ready(b).unwrap().is_none());
        assert!(fetch().apply(b).is_err());
        assert_eq!(head(b).unwrap(), previous);
        assert_eq!(
            fs::read_to_string(b.join("ignored")).unwrap(),
            "private local data"
        );
        fs::remove_file(b.join("ignored")).unwrap();
        record(b, "only-local", "local commit");
        let local = head(b).unwrap();
        assert!(matches!(fetch().apply(b).unwrap(), PullResult::Skipped(_)));
        assert_eq!(head(b).unwrap(), local);
        assert_eq!(head(server.path()).unwrap(), head(a).unwrap());
        bytes(b, &["checkout", "--detach"]).unwrap();
        assert!(auto_ready(b).unwrap().is_some());
    }

    #[test]
    fn empty_clone_receives_first_remote_commit_without_overwriting_ignored_files() {
        use std::fs;
        let server = tempfile::tempdir().unwrap();
        let cloned = tempfile::tempdir().unwrap();
        let dir = cloned.path();
        bytes(
            server.path(),
            &["init", "--bare", "--initial-branch=main", "--template="],
        )
        .unwrap();
        clone_from(
            dir,
            &fixture(server.path(), None),
            "https://github.com/sample/repo.git",
        )
        .unwrap();
        let fetch = || FetchedPull::fetch(dir, &fixture(server.path(), Some(dir))).unwrap();
        assert!(head(dir).unwrap().is_none());
        assert!(matches!(fetch().apply(dir).unwrap(), PullResult::Current));
        let published = working_tree();
        set_origin(published.path(), "sample/repo", None).unwrap();
        record(published.path(), "file", "published");
        transfer_verified(
            published.path(),
            "push",
            &fixture(server.path(), Some(published.path())),
        )
        .unwrap();
        let fetched = fetch();
        fs::create_dir_all(dir.join(".git/info")).unwrap();
        fs::write(dir.join(".git/info/exclude"), "file\n").unwrap();
        fs::write(dir.join("file"), "local").unwrap();
        assert!(fetched.apply(dir).is_err());
        assert!(head(dir).unwrap().is_none());
        assert_eq!(fs::read_to_string(dir.join("file")).unwrap(), "local");
        fs::remove_file(dir.join("file")).unwrap();
        assert!(matches!(fetched.apply(dir).unwrap(), PullResult::Updated));
        assert_eq!(head(dir).unwrap(), head(published.path()).unwrap());
        assert_eq!(fs::read_to_string(dir.join("file")).unwrap(), "published");
    }

    #[test]
    fn same_name_edits_after_first_push_and_clone_transfer_only_new_objects() {
        use std::{collections::BTreeSet, fs, sync::mpsc, time::Duration};

        let remote = tempfile::tempdir().unwrap();
        let server = remote.path();
        bytes(
            server,
            &["init", "--bare", "--initial-branch=main", "--template="],
        )
        .unwrap();
        // Keep each incoming pack so we can inspect what was actually transferred.
        bytes(server, &["config", "receive.unpackLimit", "1"]).unwrap();
        bytes(server, &["config", "receive.autogc", "false"]).unwrap();
        let packs = || -> BTreeSet<PathBuf> {
            fs::read_dir(server.join("objects/pack"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "idx"))
                .collect()
        };

        let created = working_tree();
        let first = created.path();
        set_origin(first, "sample/repo", None).unwrap();
        for file in ["selected", "unselected", "unchanged"] {
            fs::write(first.join(file), "one").unwrap();
        }
        stage(
            first,
            &["selected".into(), "unselected".into(), "unchanged".into()],
        )
        .unwrap();
        let initial = inspect_commit(first).unwrap();
        commit(first, "Initial upload", "", &initial.tree, None).unwrap();
        transfer_verified(first, "push", &fixture(server, Some(first))).unwrap();

        let cloned = tempfile::tempdir().unwrap();
        for (checkout, replacement) in [(first, "two"), (cloned.path(), "six")] {
            if checkout == cloned.path() {
                clone_from(
                    checkout,
                    &fixture(server, None),
                    "https://github.com/sample/repo.git",
                )
                .unwrap();
                identity(checkout, "Test Author", "author@example.invalid").unwrap();
            }
            let prior = packs();
            let (send, events) = mpsc::channel();
            let _monitor = watch(checkout, move |event| {
                let _ = send.send(event);
            })
            .unwrap();
            fs::write(checkout.join("selected"), replacement).unwrap();
            fs::write(checkout.join("unselected"), "working copy only").unwrap();
            events
                .recv_timeout(Duration::from_secs(20))
                .unwrap()
                .unwrap();
            let status = snapshot(checkout).unwrap();
            assert_eq!(status.changes.len(), 2);
            assert!(status
                .changes
                .iter()
                .all(|item| item.worktree == "M" && item.index == " "));
            stage(checkout, &["selected".into()]).unwrap();
            let review = inspect_commit(checkout).unwrap();
            commit(checkout, "Selected edit", "", &review.tree, None).unwrap();
            assert_eq!(snapshot(checkout).unwrap().ahead, 1);
            transfer_verified(checkout, "push", &fixture(server, Some(checkout))).unwrap();
            let status = snapshot(checkout).unwrap();
            assert_eq!(status.ahead, 0);
            assert_eq!(status.changes.len(), 1);
            assert_eq!(status.changes[0].path, "unselected");
            assert_eq!(
                line(server, &["show", "HEAD:selected"]).unwrap(),
                replacement
            );
            assert_eq!(line(server, &["show", "HEAD:unselected"]).unwrap(), "one");
            assert_eq!(line(server, &["show", "HEAD:unchanged"]).unwrap(), "one");
            let new_packs = packs();
            let received: Vec<_> = new_packs.difference(&prior).collect();
            assert_eq!(received.len(), 1);
            let listing = line(
                server,
                &["verify-pack", "-v", received[0].to_str().unwrap()],
            )
            .unwrap();
            let ids: BTreeSet<_> = listing
                .lines()
                .filter_map(|row| {
                    let mut fields = row.split_whitespace();
                    let id = fields.next()?;
                    matches!(fields.next()?, "commit" | "tree" | "blob" | "tag")
                        .then_some(id.to_owned())
                })
                .collect();
            let expected = ["HEAD", "HEAD^{tree}", "HEAD:selected"]
                .map(|revision| line(server, &["rev-parse", revision]).unwrap());
            assert_eq!(ids, BTreeSet::from(expected));
        }
    }
}
