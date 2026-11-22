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

    fn fetch(&self) -> Result<()> {
        self.run(
            &[
                "fetch",
                "--no-recurse-submodules",
                "--no-tags",
                "--",
                &self.url,
                "+refs/heads/*:refs/heads/*",
                "refs/tags/*:refs/tags/*",
            ],
            true,
            None,
        )?;
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
    bridge.fetch()?;
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

fn fetch_into(dir: &Path, bridge: &Transport<'_>) -> Result<()> {
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
    bridge.fetch()?;
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
        checked(execute(
            command().current_dir(dir).args(["update-ref", "--stdin"]),
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
        return fetch_into(dir, bridge);
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
            fetch_into(dir, bridge)?;
            // A remote-deleted branch may still have a local tracking ref. Use
            // only the tip actually received by this fetch.
            let id = utf8(
                &bridge
                    .run(
                        &[
                            "rev-parse",
                            "--verify",
                            &format!("refs/heads/{target}^{{commit}}"),
                        ],
                        false,
                        None,
                    )
                    .map_err(|_| "The tracking branch no longer exists on GitHub.")?,
            )?
            .trim()
            .to_owned();
            if current_branch(dir)?.as_deref() != Some(&local) || !changes(dir)?.is_empty() {
                return Err(
                    "The local branch or worktree changed during fetch. Inspect it and retry."
                        .into(),
                );
            }
            bytes(dir, &["merge", "--ff-only", "--no-edit", "--", &id])?;
            if !bytes(dir, &["ls-files", "-u", "-z"])?.is_empty() {
                return Err("Unresolved conflicts remain in the worktree.".into());
            }
            Ok(())
        }
        _ => Err("Unknown Git transfer operation.".into()),
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
}
