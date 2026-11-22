use super::*;
use std::{fs, sync::mpsc, time::Duration};
use tempfile::{tempdir, TempDir};

pub(super) fn working_tree() -> TempDir {
    let folder = tempdir().unwrap();
    init(folder.path()).unwrap();
    identity(folder.path(), "Test Author", "author@example.invalid").unwrap();
    folder
}

pub(super) fn record(dir: &Path, path: &str, data: &str) {
    fs::write(dir.join(path), data).unwrap();
    stage(dir, &[path.to_owned()]).unwrap();
    let review = inspect_commit(dir).unwrap();
    commit(dir, "Sample commit", "Paragraph two", &review.tree, None).unwrap();
}

#[test]
fn commits_fill_missing_git_identity_from_the_associated_account() {
    let folder = working_tree();
    let dir = folder.path();
    for (index, (name, email, id, login)) in [
        ("", "", "42", "octocat"),
        ("Custom Name", "", "7", "another-account"),
        ("", "user@example.invalid", "42", "octocat"),
        ("Custom Name", "user@example.invalid", "42", "octocat"),
    ]
    .into_iter()
    .enumerate()
    {
        // Empty local values shadow any global Git identity on the test machine.
        bytes(dir, &["config", "--local", "user.name", name]).unwrap();
        bytes(dir, &["config", "--local", "user.email", email]).unwrap();
        let account = AccountEntry {
            id: id.into(),
            login: login.into(),
        };
        fs::write(dir.join("file"), index.to_string()).unwrap();
        stage(dir, &["file".into()]).unwrap();
        let review = inspect_commit(dir).unwrap();
        commit(dir, "Record identity", "", &review.tree, Some(&account)).unwrap();

        let expected_name = if name.is_empty() { login } else { name };
        let expected_email = if email.is_empty() {
            format!("{id}+{login}@users.noreply.github.com")
        } else {
            email.to_owned()
        };
        assert_eq!(
            line(dir, &["log", "-1", "--format=%an|%ae|%cn|%ce"]).unwrap(),
            format!("{expected_name}|{expected_email}|{expected_name}|{expected_email}")
        );
        assert_eq!(
            line(dir, &["config", "--local", "user.name"]).unwrap(),
            name
        );
        assert_eq!(
            line(dir, &["config", "--local", "user.email"]).unwrap(),
            email
        );
    }
}

#[test]
fn stage_only_selected_paths_and_preserve_partial_changes() {
    let repo = working_tree();
    let d = repo.path();
    fs::create_dir(d.join("src")).unwrap();
    fs::write(d.join("src/main.rs"), "one").unwrap();
    fs::write(d.join("other"), "two").unwrap();
    let selected = vec!["src/main.rs".into()];
    stage(d, &selected).unwrap();
    unstage(d, &selected).unwrap();
    assert_eq!(
        changes(d)
            .unwrap()
            .iter()
            .filter(|c| c.index == "?")
            .count(),
        2
    );
    stage(d, &selected).unwrap();
    let review = inspect_commit(d).unwrap();
    assert!(review.warnings.is_empty());
    fs::write(d.join("src/main.rs"), "not staged").unwrap();
    commit(d, "First commit", "Second paragraph", &review.tree, None).unwrap();
    let state = snapshot(d).unwrap();
    assert_eq!(state.history[0].title, "First commit");
    assert_eq!(state.history[0].author, "Test Author");
    assert_eq!(state.ahead, 1);
    assert_eq!(line(d, &["show", "HEAD:src/main.rs"]).unwrap(), "one");
    assert_eq!(state.changes.len(), 2);
    assert_eq!(
        state
            .changes
            .iter()
            .find(|v| v.path == "src/main.rs")
            .unwrap()
            .worktree,
        "M"
    );
    assert!(line(d, &["show", "HEAD:other"]).is_err());
    assert!(root(&d.join("src")).is_err());
}

#[test]
fn index_review_uses_immutable_blob_and_rejects_changed_index() {
    let repo = working_tree();
    let d = repo.path();
    let mark = ["sensitive", "-sample", "-value"].concat();
    let candidate = format!("{} = '{}'", ["pass", "word"].concat(), mark);
    fs::write(d.join("config"), candidate).unwrap();
    stage(d, &["config".into()]).unwrap();
    fs::write(d.join("config"), "benign replacement").unwrap();
    let review = inspect_commit(d).unwrap();
    assert_eq!(review.warnings.len(), 1);
    assert!(!review.warnings[0].contains(&mark));
    stage(d, &["config".into()]).unwrap();
    assert!(commit(d, "old review", "", &review.tree, None).is_err());
    assert!(head(d).unwrap().is_none());
    fs::write(d.join(".env"), "").unwrap();
    stage(d, &[".env".into()]).unwrap();
    assert!(inspect_commit(d)
        .unwrap()
        .warnings
        .iter()
        .any(|v| v.contains(".env")));
}

#[test]
fn literal_paths_renames_deleted_files_and_ignored_files() {
    let repo = working_tree();
    let d = repo.path();
    record(d, "[literal]", "initial");
    fs::rename(d.join("[literal]"), d.join("renamed")).unwrap();
    stage(d, &["[literal]".into(), "renamed".into()]).unwrap();
    let state = changes(d).unwrap();
    assert_eq!(state[0].index, "R");
    assert_eq!(state[0].original.as_deref(), Some("[literal]"));
    unstage(d, &["renamed".into()]).unwrap();
    assert!(changes(d)
        .unwrap()
        .iter()
        .all(|c| c.index == " " || c.index == "?"));
    fs::write(d.join(".gitignore"), "ignored\n").unwrap();
    fs::write(d.join("ignored"), "excluded").unwrap();
    assert!(changes(d).unwrap().iter().all(|c| c.path != "ignored"));
}

#[test]
fn branches_preserve_unmerged_commits_and_reject_overwrites() {
    let repo = working_tree();
    let d = repo.path();
    record(d, "tracked", "base");
    branch(d, "create", "topic", "").unwrap();
    branch(d, "switch", "topic", "").unwrap();
    record(d, "tracked", "topic");
    branch(d, "rename", "topic", "renamed").unwrap();
    branch(d, "switch", "main", "").unwrap();
    assert!(branch(d, "delete", "renamed", "").is_err());
    fs::write(d.join("tracked"), "local working copy").unwrap();
    assert!(branch(d, "switch", "renamed", "").is_err());
    assert_eq!(
        fs::read_to_string(d.join("tracked")).unwrap(),
        "local working copy"
    );
    branch(d, "create", "merged", "").unwrap();
    branch(d, "delete", "merged", "").unwrap();
}

#[test]
fn filesystem_monitor_detects_copy_replace_rename_and_delete_bursts() {
    let repo = working_tree();
    let d = repo.path();
    fs::create_dir(d.join("nested")).unwrap();
    let (send, receive) = mpsc::channel();
    let _watch = watch(d, move |evt| {
        let _ = send.send(evt);
    })
    .unwrap();
    for i in 0..150 {
        fs::write(d.join(format!("nested/file{i}")), "a").unwrap();
    }
    let seen = || {
        receive
            .recv_timeout(Duration::from_secs(20))
            .unwrap()
            .unwrap()
    };
    seen();
    assert_eq!(changes(d).unwrap().len(), 150);
    record(d, "tracked", "initial");
    fs::write(d.join("replacement"), "new version").unwrap();
    fs::rename(d.join("replacement"), d.join("tracked")).unwrap();
    seen();
    assert!(changes(d)
        .unwrap()
        .iter()
        .any(|c| c.path == "tracked" && c.worktree == "M"));
    fs::rename(d.join("tracked"), d.join("moved")).unwrap();
    seen();
    assert!(changes(d)
        .unwrap()
        .iter()
        .any(|c| c.path == "tracked" && c.worktree == "D"));
    fs::remove_file(d.join("moved")).unwrap();
    seen();
    assert!(!snapshot(&root(d).unwrap())
        .unwrap()
        .changes
        .iter()
        .any(|c| c.path == "moved"));
}

#[test]
fn validates_fetch_and_push_origin_and_preserves_local_configuration() {
    let repo = working_tree();
    let d = repo.path();
    set_origin(d, "demo/project", None).unwrap();
    assert_eq!(origin(d).unwrap().as_deref(), Some("demo/project"));
    assert!(transfer(d, "push", "different/project", "test-only").is_err());
    set_origin(d, "demo/renamed", Some("demo/project")).unwrap();
    assert!(set_origin(d, "demo/other", Some("demo/project")).is_err());
    bytes(
        d,
        &[
            "config",
            "remote.origin.pushurl",
            "https://elsewhere.invalid/repo",
        ],
    )
    .unwrap();
    assert_eq!(origin(d).unwrap(), None);
    assert_eq!(github_target("https://user@github.com/demo/repo"), None);
    assert_eq!(
        github_target("git@github.com:demo/repo.git").as_deref(),
        Some("demo/repo")
    );
}
