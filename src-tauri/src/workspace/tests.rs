use super::*;
use crate::github::tests::{installation, listing, login, remote, reply, service, Reply};
use serde_json::{json, Value};
use tempfile::TempDir;

fn database() -> (TempDir, Database) {
    let temp = tempfile::tempdir().unwrap();
    let mut db = Database::load(&temp.path().join("settings")).unwrap();
    db.change(|s| s.active_account = Some("1".into())).unwrap();
    (temp, db)
}
fn creation(parent: &Path) -> Creation {
    Creation {
        name: "sample".into(),
        description: "test".into(),
        parent: parent.into(),
        readme: true,
        license: "mit".into(),
        gitignore: "Rust".into(),
    }
}
fn create_script() -> Vec<Reply> {
    vec![reply("POST", "/user/repos", remote("sample", true))]
}
fn get_script() -> Vec<Reply> {
    vec![
        installation(),
        listing("sample"),
        reply("GET", "/repos/fixture/sample", remote("sample", true)),
    ]
}
fn account(api: &Github) {
    login(api, 1, "test-session", u64::MAX);
}
fn public() -> Metadata {
    Metadata {
        name: "sample".into(),
        description: "test".into(),
        private: false,
    }
}

#[test]
fn description_limit_counts_unicode_characters() {
    assert!(validate_description("").is_ok());
    assert!(validate_description(&"😀".repeat(350)).is_ok());
    assert!(validate_description(&"😀".repeat(351)).is_err());
}

#[test]
fn creation_preserves_paths_private_remote_and_unstaged_options() {
    let (temp, mut db) = database();
    let (api, http) = service(create_script());
    account(&api);
    let repo = create(&mut db, &api, creation(temp.path())).unwrap().repo;
    let reqs = http.join().unwrap();
    let sent: Value = serde_json::from_str(&reqs[0].body).unwrap();
    assert_eq!(sent["private"], true);
    assert_eq!(sent["auto_init"], false);
    assert_eq!(
        git::origin(&repo.path).unwrap().as_deref(),
        Some("fixture/sample")
    );
    let status = git::snapshot(&repo.path).unwrap();
    assert!(status.history.is_empty());
    assert_eq!(status.changes.len(), 3);
    assert!(status.changes.iter().all(|c| c.index == "?"));
    assert!(repo.first_push);
    assert!(reserve(temp.path(), "sample").is_err());
    assert!(repo.path.join("LICENSE").exists());
    assert!(open_repo(&mut db, &api, &repo.path).is_ok());
    db.change(|d| d.active_account = Some("2".into())).unwrap();
    assert!(db.repo(&repo.id).is_err());
    assert!(open_repo(&mut db, &api, &repo.path).is_err());
}

#[test]
fn partial_creation_retains_local_copy_and_allows_empty_initialization() {
    let (temp, mut db) = database();
    let mut script = create_script();
    script[0].status = 422;
    script[0].json = json!({});
    let (api, http) = service(script);
    account(&api);
    let mut input = creation(temp.path());
    input.readme = false;
    input.license.clear();
    input.gitignore.clear();
    assert!(create(&mut db, &api, input)
        .unwrap()
        .warning
        .unwrap()
        .contains("retained"));
    let copy = &db.data.repos[0];
    assert!(copy.full_name.is_none());
    assert!(copy.path.join(".git").exists());
    assert!(git::snapshot(&copy.path).unwrap().changes.is_empty());
    http.join().unwrap();
    for license in [
        "mit",
        "apache-2.0",
        "gpl-3.0",
        "lgpl-3.0",
        "agpl-3.0",
        "mpl-2.0",
        "unlicense",
        "bsd-3-clause",
    ] {
        assert!(license_text(license).unwrap().unwrap().len() > 900);
    }
    for ignore in ["Rust", "Node", "Python"] {
        assert!(!ignore_text(ignore).unwrap().unwrap().is_empty());
    }
    for unsafe_name in ["..", "a/b", "con", ".git", "LPT1.txt", "repo."] {
        assert!(reserve(temp.path(), unsafe_name).is_err());
    }
}

#[test]
fn recovery_creates_or_connects_the_remote_without_touching_local_history() {
    for mode in ["create", "existing"] {
        let (temp, mut db) = database();
        let mut denied = create_script();
        denied[0].status = 403;
        denied.extend(if mode == "create" {
            create_script()
        } else {
            get_script()
        });
        let (api, http) = service(denied);
        account(&api);
        let created = create(&mut db, &api, creation(temp.path())).unwrap();
        assert!(created
            .warning
            .unwrap()
            .contains("GitHub denied repository creation"));
        let repo = db.repo(&created.repo.id).unwrap();
        let run = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(&repo.path)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success());
            output.stdout
        };
        run(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "Local work",
        ]);
        let head = run(&["rev-parse", "HEAD"]);
        let target = if mode == "create" {
            "sample"
        } else {
            "fixture/sample"
        };
        let linked = connect(&mut db, &api, &repo, mode, target).unwrap();
        assert_eq!(head, run(&["rev-parse", "HEAD"]));
        assert_eq!(
            Some("fixture/sample"),
            git::origin(&repo.path).unwrap().as_deref()
        );
        assert_eq!(linked.id, repo.id);
        assert_eq!(linked.remote_id, Some(100));
        assert_eq!(db.repo(&repo.id).unwrap().first_push, mode == "create");
        assert!(connect(&mut db, &api, &linked, mode, target).is_err());
        http.join().unwrap();
    }
}

#[test]
fn connection_verifies_privacy_write_access_and_existing_origin() {
    let (temp, mut db) = database();
    let mut script = Vec::new();
    for (private, push) in [(false, true), (true, false), (true, true), (true, true)] {
        let mut item = remote("sample", private);
        item["permissions"]["push"] = push.into();
        script.extend([
            installation(),
            listing("sample"),
            reply("GET", "/repos/fixture/sample", item),
        ]);
    }
    let (api, http) = service(script);
    account(&api);
    let path = temp.path().join("checkout");
    fs::create_dir(&path).unwrap();
    git::init(&path).unwrap();
    let repo = open_repo(&mut db, &api, &path).unwrap();
    for expected in ["private", "write access"] {
        assert!(connect(&mut db, &api, &repo, "existing", "fixture/sample")
            .err()
            .unwrap()
            .contains(expected));
        assert!(git::origin(&path).unwrap().is_none());
        assert!(db.repo(&repo.id).unwrap().full_name.is_none());
    }
    git::set_origin(&path, "fixture/other", None).unwrap();
    assert!(connect(&mut db, &api, &repo, "existing", "fixture/sample").is_err());
    assert!(connect(&mut db, &api, &repo, "create", "sample").is_err());
    assert_eq!(
        git::origin(&path).unwrap().as_deref(),
        Some("fixture/other")
    );
    // Origin already written but the application was interrupted before recording it.
    git::set_origin(&path, "fixture/sample", Some("fixture/other")).unwrap();
    assert!(connect(&mut db, &api, &repo, "existing", "fixture/sample").is_ok());
    http.join().unwrap();
}

#[test]
fn publication_requires_confirmation_and_verifies_push_before_visibility_change() {
    for (push_ok, visibility_ok) in [(false, false), (true, false), (true, true)] {
        let (temp, mut db) = database();
        let mut script = create_script();
        script.extend(get_script()); // unconfirmed push
        script.extend(get_script()); // first confirmed attempt
        script.extend(get_script()); // recheck private before transfer
        if push_ok {
            script.extend(get_script());
            script.extend([installation(), listing("sample")]);
            let mut patch = reply("PATCH", "/repos/fixture/sample", remote("sample", false));
            if !visibility_ok {
                patch.status = 403;
            }
            script.push(patch);
        }
        let (api, http) = service(script);
        account(&api);
        let mut repo = create(&mut db, &api, creation(temp.path())).unwrap().repo;
        assert!(
            sync_using(&mut db, &api, &mut repo, "push", None, |_, _, _, _| panic!(
                "unconfirmed transfer"
            ))
            .is_err()
        );
        let result = sync_using(
            &mut db,
            &api,
            &mut repo,
            "push",
            Some(public()),
            |_, action, full, _| {
                assert_eq!(action, "push");
                assert_eq!(full, "fixture/sample");
                if push_ok {
                    Ok(())
                } else {
                    Err("rejected".into())
                }
            },
        );
        assert_eq!(result.is_ok(), push_ok && visibility_ok);
        assert_eq!(db.repo(&repo.id).unwrap().first_push, !push_ok);
        let requests = http.join().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.body.contains("\"private\":false"))
                .count(),
            usize::from(push_ok)
        );
    }
}

#[test]
fn rename_visibility_and_both_delete_check_exact_targets() {
    let (temp, mut db) = database();
    let mut script = create_script();
    script.extend(get_script());
    script.extend([
        installation(),
        listing("sample"),
        reply("PATCH", "/repos/fixture/sample", remote("renamed", false)),
    ]);
    script.extend([
        installation(),
        listing("renamed"),
        reply("GET", "/repos/fixture/renamed", remote("renamed", false)),
        installation(),
        listing("renamed"),
        reply("DELETE", "/repos/fixture/renamed", Value::Null),
    ]);
    script.last_mut().unwrap().status = 204;
    let (api, http) = service(script);
    account(&api);
    let mut repo = create(&mut db, &api, creation(temp.path())).unwrap().repo;
    repo.first_push = false;
    db.put(&repo).unwrap();
    update(
        &mut db,
        &api,
        &mut repo,
        Metadata {
            name: "renamed".into(),
            description: "new description".into(),
            private: false,
        },
    )
    .unwrap();
    assert_eq!(
        git::origin(&repo.path).unwrap().as_deref(),
        Some("fixture/renamed")
    );
    assert!(delete(&mut db, &api, &mut repo, "both", "sample").is_err());
    assert!(repo.path.is_dir());
    delete(&mut db, &api, &mut repo, "both", "fixture/renamed").unwrap();
    assert!(!repo.path.exists() && db.data.repos.is_empty());
    let requests = http.join().unwrap();
    assert!(requests.iter().any(|r| r.body.contains("new description")));
}

#[test]
fn removal_keeps_files_and_replaced_checkouts_are_never_deleted() {
    let (temp, mut db) = database();
    let (api, http) = service(vec![]);
    let path = temp.path().join("work");
    fs::create_dir(&path).unwrap();
    git::init(&path).unwrap();
    let mut repo = open_repo(&mut db, &api, &path).unwrap();
    fs::rename(&path, temp.path().join("moved")).unwrap();
    fs::create_dir(&path).unwrap();
    git::init(&path).unwrap();
    assert!(delete(&mut db, &api, &mut repo, "local", "work").is_err());
    delete(&mut db, &api, &mut repo, "remove", "work").unwrap();
    assert!(path.join(".git").is_dir());
    assert!(db.data.repos.is_empty());
    http.join().unwrap();
}

#[test]
fn account_change_is_explicit_and_relocation_checks_checkout_identity() {
    use crate::storage::AccountEntry;
    let (temp, mut db) = database();
    db.change(|d| {
        d.accounts.push(AccountEntry {
            id: "2".into(),
            login: "another".into(),
        })
    })
    .unwrap();
    let (api, http) = service(vec![]);
    let old = temp.path().join("old");
    fs::create_dir(&old).unwrap();
    git::init(&old).unwrap();
    let mut repo = open_repo(&mut db, &api, &old).unwrap();
    let moved = temp.path().join("moved");
    fs::rename(&old, &moved).unwrap();
    fs::create_dir(&old).unwrap();
    git::init(&old).unwrap();
    assert!(relocate(&mut db, &mut repo, &old).is_err());
    relocate(&mut db, &mut repo, &moved).unwrap();
    assert_eq!(db.repo(&repo.id).unwrap().path, moved);
    assert!(reassociate(&mut db, &api, &mut repo, 3).is_err());
    reassociate(&mut db, &api, &mut repo, 2).unwrap();
    assert!(db.repo(&repo.id).is_err());
    db.change(|d| d.active_account = Some("2".into())).unwrap();
    assert_eq!(db.repo(&repo.id).unwrap().account_id, "2");
    http.join().unwrap();
}
