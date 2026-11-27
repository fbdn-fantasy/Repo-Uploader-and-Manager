//! The desktop process owns a single scheduler. Network traffic for one checkout
//! never holds the workspace mutex; only its own mutations are reserved.
use super::{git, workspace, Service, Session};
use crate::storage::{Database, Repo, Result, Settings};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap},
    sync::{mpsc, Arc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};

const INTERVAL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    id: String,
    #[serde(skip)]
    account: String,
    status: &'static str,
    message: String,
    checked_at: u64,
}

pub enum Wake {
    All,
    Account(String),
}

#[derive(Default)]
pub struct State {
    generation: u64,
    pub busy: Option<String>,
    items: HashMap<String, Item>,
}

impl State {
    pub fn forget(&mut self, id: &str) {
        self.items.remove(id);
    }

    pub fn set_enabled(&mut self, db: &mut Database, enabled: bool) -> Result<bool> {
        if db.data.auto_sync_enabled == enabled {
            return Ok(false);
        }
        db.change(|settings| settings.auto_sync_enabled = enabled)?;
        self.generation = self.generation.wrapping_add(1);
        self.items.retain(|_, item| item.status != "checking");
        Ok(true)
    }

    fn enabled_for(&self, settings: &Settings, generation: u64) -> bool {
        settings.auto_sync_enabled && self.generation == generation
    }

    pub fn visible(&self, settings: &Settings) -> Vec<&Item> {
        settings
            .repos
            .iter()
            .filter(|repo| {
                Some(&repo.account_id) == settings.active_account.as_ref()
                    && repo.full_name.is_some()
            })
            .filter_map(|repo| {
                self.items
                    .get(&repo.id)
                    .filter(|item| item.account == repo.account_id)
            })
            .collect()
    }
}

fn candidates(settings: &Settings, accounts: Option<&BTreeSet<String>>) -> Vec<Repo> {
    if !settings.auto_sync_enabled {
        return Vec::new();
    }
    settings
        .repos
        .iter()
        .filter(|repo| {
            repo.full_name.is_some()
                && accounts.is_none_or(|selected| selected.contains(&repo.account_id))
                && settings
                    .accounts
                    .iter()
                    .any(|account| account.id == repo.account_id)
                && repo.path.is_dir()
                && workspace::verify_local(repo).is_ok()
        })
        .cloned()
        .collect()
}

fn saved(session: &Session, repo: &Repo) -> bool {
    session
        .db
        .data
        .accounts
        .iter()
        .any(|account| account.id == repo.account_id)
        && session.db.data.repos.iter().any(|r| {
            r.id == repo.id
                && r.account_id == repo.account_id
                && r.path == repo.path
                && r.identity == repo.identity
                && r.full_name == repo.full_name
                && r.first_push == repo.first_push
                && r.remote_id == repo.remote_id
                && r.previous_origin == repo.previous_origin
        })
}

impl Service {
    fn auto_batch(&self, app: &AppHandle, accounts: Option<&BTreeSet<String>>) {
        // Git and filesystem checks can be slow on external disks. Inspect the
        // saved paths outside the UI lock and never discover other checkouts.
        let Ok((generation, settings)) =
            self.with_session(|s| Ok((s.auto.generation, s.db.data.clone())))
        else {
            return;
        };
        let repos = candidates(&settings, accounts);
        let ids: BTreeSet<_> = repos.iter().map(|r| r.id.as_str()).collect();
        let removed = self
            .with_session(|s| {
                if !s.auto.enabled_for(&s.db.data, generation) {
                    return Ok(false);
                }
                let mut removed = false;
                s.auto.items.retain(|id, item| {
                    let keep = accounts.is_some_and(|list| !list.contains(&item.account))
                        || ids.contains(id.as_str());
                    removed |= !keep;
                    keep
                });
                Ok(removed)
            })
            .unwrap_or(false);
        if removed {
            let _ = app.emit("auto-sync", ());
        }
        for mut repo in repos {
            if !self.report(
                app,
                generation,
                &repo,
                "checking",
                "Checking GitHub…".into(),
            ) {
                break;
            }
            let result = self.auto_pull(generation, &mut repo);
            let (status, message) = match result {
                Ok(git::PullResult::Current) => (
                    "current",
                    "All incoming commits are present locally.".into(),
                ),
                Ok(git::PullResult::Updated) => {
                    let _ = app.emit("repo-change", serde_json::json!({"id":repo.id}));
                    ("updated", "Pulled new commits from GitHub.".into())
                }
                Ok(git::PullResult::Skipped(why)) => ("skipped", why.into()),
                Err(message) => ("error", message),
            };
            self.report(app, generation, &repo, status, message);
        }
    }

    fn report(
        &self,
        app: &AppHandle,
        generation: u64,
        repo: &Repo,
        status: &'static str,
        message: String,
    ) -> bool {
        let changed = self
            .with_session(|session| {
                if !session.auto.enabled_for(&session.db.data, generation) {
                    return Ok(false);
                }
                if saved(session, repo) {
                    session.auto.items.insert(
                        repo.id.clone(),
                        Item {
                            id: repo.id.clone(),
                            account: repo.account_id.clone(),
                            status,
                            message,
                            checked_at: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                        },
                    );
                } else {
                    session.auto.items.remove(&repo.id);
                }
                Ok(true)
            })
            .unwrap_or(false);
        if changed {
            let _ = app.emit("auto-sync", ());
        }
        changed
    }

    fn auto_pull(&self, generation: u64, repo: &mut Repo) -> Result<git::PullResult> {
        use git::PullResult::Skipped;
        if repo.first_push {
            return Ok(Skipped("Waiting for the first manual push."));
        }
        workspace::verify_local(repo)?;
        if let Some(reason) = git::auto_ready(&repo.path)? {
            return Ok(Skipped(reason));
        }
        let full = repo
            .full_name
            .clone()
            .ok_or("No GitHub destination is associated.")?;
        if !git::origin(&repo.path)?.is_some_and(|origin| origin.eq_ignore_ascii_case(&full)) {
            return Ok(Skipped(
                "The configured origin differs from the saved GitHub repository.",
            ));
        }
        let account = repo
            .account_id
            .parse()
            .map_err(|_| "Invalid associated account.")?;
        if !self
            .with_session(|s| Ok(s.auto.enabled_for(&s.db.data, generation) && saved(s, repo)))?
        {
            return Ok(Skipped("The sync setting or saved repository changed."));
        }
        let token = self.github.token(account)?;
        let remote_id = self
            .github
            .sync_repository_id(account, &token, &full, repo.remote_id)?;
        let _transfer = self
            .install_gate
            .lock()
            .map_err(|_| "Restart the app to resume synchronization.")?;
        let reserved = self.with_session(|session| {
            if !session.auto.enabled_for(&session.db.data, generation) || !saved(session, repo) {
                return Ok(false);
            }
            workspace::verify_local(repo)?;
            if repo.remote_id.is_none() {
                repo.remote_id = Some(remote_id);
                session.db.put(repo)?;
            }
            session.auto.busy = Some(repo.id.clone());
            Ok(true)
        })?;
        if !reserved {
            return Ok(Skipped("The saved repository or its account changed."));
        }
        let result = git::prepare_auto_pull(&repo.path, &full, &token);
        self.with_session(|session| {
            session.auto.busy = None;
            if !session.auto.enabled_for(&session.db.data, generation) || !saved(session, repo) {
                return Ok(Skipped("The saved repository or its account changed."));
            }
            let fetched = result?;
            workspace::verify_local(repo)?;
            if !git::origin(&repo.path)?.is_some_and(|origin| origin.eq_ignore_ascii_case(&full)) {
                return Ok(Skipped(
                    "The configured origin changed during synchronization.",
                ));
            }
            let outcome = fetched.apply(&repo.path)?;
            if matches!(outcome, git::PullResult::Updated)
                && session
                    .review
                    .as_ref()
                    .is_some_and(|review| review.0 == repo.id)
            {
                session.review = None;
            }
            Ok(outcome)
        })
    }
}

// If the machine sleeps or a fetch exceeds one period, coalesce missed checks.
fn next_due(previous: Instant, now: Instant) -> Instant {
    if previous + INTERVAL > now {
        previous + INTERVAL
    } else {
        now + INTERVAL
    }
}

pub fn start(
    service: Arc<Service>,
    app: AppHandle,
    requests: mpsc::Receiver<Wake>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("repository-sync".into())
        .spawn(move || {
            let mut due = Instant::now();
            loop {
                if Instant::now() >= due {
                    service.auto_batch(&app, None);
                    due = next_due(due, Instant::now());
                }
                match requests.recv_timeout(due.saturating_duration_since(Instant::now())) {
                    Ok(request) => {
                        let mut accounts = BTreeSet::new();
                        let mut all = false;
                        for wake in std::iter::once(request).chain(requests.try_iter()) {
                            match wake {
                                Wake::All => all = true,
                                Wake::Account(account) => {
                                    accounts.insert(account);
                                }
                            }
                        }
                        service.auto_batch(&app, if all { None } else { Some(&accounts) });
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::AccountEntry;

    #[test]
    fn old_settings_default_on_and_toggling_invalidates_in_flight_checks() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join("workspace.json"),
            r#"{"accounts":[],"activeAccount":null,"repos":[],"theme":"dark"}"#,
        )
        .unwrap();
        let mut db = Database::load(folder.path()).unwrap();
        let mut state = State::default();
        assert!(db.data.auto_sync_enabled);
        let running = state.generation;
        assert!(state.enabled_for(&db.data, running));
        assert!(state.set_enabled(&mut db, false).unwrap());
        assert!(!state.enabled_for(&db.data, running));
        assert!(
            !Database::load(folder.path())
                .unwrap()
                .data
                .auto_sync_enabled
        );
        assert!(!state.set_enabled(&mut db, false).unwrap());
        assert!(state.set_enabled(&mut db, true).unwrap());
        assert!(!state.enabled_for(&db.data, running));
        assert!(state.enabled_for(&db.data, state.generation));
        let restored = Database::load(folder.path()).unwrap();
        assert!(restored.data.auto_sync_enabled);
        assert_eq!(restored.data.theme, "dark");
    }

    #[test]
    fn scheduler_starts_immediately_and_coalesces_missed_intervals() {
        let start = Instant::now();
        let due = next_due(start, start + Duration::from_secs(2));
        assert_eq!(due.duration_since(start), Duration::from_secs(1800));
        let resume = start + Duration::from_secs(4 * 1800);
        assert_eq!(next_due(due, resume), resume + INTERVAL);
    }

    #[test]
    fn verifies_with_associated_account_and_rejects_replaced_remote_before_git_transfer() {
        use crate::github::tests::{login, remote, reply, service};
        use crate::storage::{checkout_identity, Database};
        let temp = tempfile::tempdir().unwrap();
        let mut db = Database::load(&temp.path().join("config")).unwrap();
        let path = temp.path().join("sample");
        std::fs::create_dir(&path).unwrap();
        git::init(&path).unwrap();
        git::identity(&path, "Example", "example@example.invalid").unwrap();
        std::fs::write(path.join("file"), "original").unwrap();
        git::stage(&path, &["file".into()]).unwrap();
        git::commit(
            &path,
            "initial",
            "",
            &git::inspect_commit(&path).unwrap().tree,
            None,
        )
        .unwrap();
        git::set_origin(&path, "fixture/sample", None).unwrap();
        let mut repo = Repo {
            id: "sample".into(),
            account_id: "2".into(),
            identity: checkout_identity(&path).unwrap(),
            path,
            name: "sample".into(),
            full_name: Some("fixture/sample".into()),
            remote_id: Some(9000),
            previous_origin: None,
            first_push: true,
        };
        db.data.active_account = Some("1".into());
        db.data.accounts = ["1", "2"]
            .map(|id| AccountEntry {
                id: id.into(),
                login: "fixture".into(),
            })
            .into();
        db.put(&repo).unwrap();
        let (github, server) = service(vec![reply(
            "GET",
            "/repos/fixture/sample",
            remote("sample", true),
        )]);
        login(&github, 2, "associated-fixture", u64::MAX);
        let (auto_wake, _) = mpsc::channel();
        let app = Service {
            github,
            updater: Default::default(),
            auto_wake,
            install_gate: Default::default(),
            session: std::sync::Mutex::new(Ok(Session {
                db,
                auto: State::default(),
                watcher: None,
                review: None,
            })),
        };
        assert!(matches!(
            app.auto_pull(0, &mut repo).unwrap(),
            git::PullResult::Skipped(_)
        ));
        repo.first_push = false;
        app.with_session(|s| s.db.put(&repo)).unwrap();
        assert!(app
            .auto_pull(0, &mut repo)
            .err()
            .unwrap()
            .contains("repository at this name has changed"));
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests.iter().all(|r| r
            .headers
            .to_ascii_lowercase()
            .contains("authorization: bearer associated-fixture")));
        app.with_session(|s| {
            assert_eq!(s.db.data.active_account.as_deref(), Some("1"));
            assert!(s.auto.busy.is_none());
            assert_eq!(s.db.data.repos[0].remote_id, Some(9000));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn batch_includes_signed_in_associations_and_limits_status_to_active_account() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.active_account = Some("1".into());
        let mut state = State::default();
        for (id, connected, signed_in) in [
            ("1", true, true),
            ("2", true, true),
            ("3", true, false),
            ("4", false, true),
        ] {
            if signed_in {
                settings.accounts.push(AccountEntry {
                    id: id.into(),
                    login: "fixture".into(),
                });
            }
            let path = temp.path().join(id);
            std::fs::create_dir(&path).unwrap();
            git::init(&path).unwrap();
            let identity = crate::storage::checkout_identity(&path).unwrap();
            settings.repos.push(Repo {
                id: id.into(),
                account_id: id.into(),
                path,
                name: "sample".into(),
                full_name: connected.then(|| "sample/repo".into()),
                first_push: false,
                remote_id: Some(10),
                identity,
                previous_origin: None,
            });
            state.items.insert(
                id.into(),
                Item {
                    id: id.into(),
                    account: id.into(),
                    status: "current",
                    message: "current".into(),
                    checked_at: 1,
                },
            );
        }
        // A valid Git folder beside the saved ones is never discovered. A
        // missing saved folder and a replacement at an old path are ineligible.
        let other = temp.path().join("unregistered");
        std::fs::create_dir(&other).unwrap();
        git::init(&other).unwrap();
        let mut missing = settings.repos[0].clone();
        missing.id = "missing".into();
        missing.path = temp.path().join("does-not-exist");
        settings.repos.push(missing.clone());
        missing.id = "replaced".into();
        missing.path = other;
        settings.repos.push(missing.clone());
        missing.id = "file".into();
        missing.path = temp.path().join("regular-file");
        std::fs::write(&missing.path, "file").unwrap();
        settings.repos.push(missing);
        let ids = |list: Vec<Repo>| list.into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids(candidates(&settings, None)), ["1", "2"]);
        assert_eq!(
            ids(candidates(&settings, Some(&BTreeSet::from(["2".into()])))),
            ["2"]
        );
        assert_eq!(
            state
                .visible(&settings)
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["1"]
        );
        settings.auto_sync_enabled = false;
        assert!(candidates(&settings, None).is_empty());
        assert_eq!(settings.active_account.as_deref(), Some("1"));
    }
}
