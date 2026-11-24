#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autosync;
mod git;
mod github;
mod storage;
mod updater;
mod window;
mod workspace;

use github::Github;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use storage::{AccountEntry, Database, Repo, Result};
use tauri::{Emitter, Manager};

struct Session {
    db: Database,
    auto: autosync::State,
    watcher: Option<git::Watcher>,
    review: Option<(String, String, bool)>,
}

struct Service {
    github: Github,
    updater: Mutex<updater::Updater>,
    auto_wake: std::sync::mpsc::Sender<autosync::Wake>,
    install_gate: Mutex<()>,
    // Serialize repository mutations and account switches across IPC workers.
    session: Mutex<Result<Session>>,
}

fn field<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing or invalid field: {name}."))
}

fn number(args: &Value, name: &str) -> Result<u64> {
    field(args, name)?
        .parse()
        .map_err(|_| "Invalid account or session identifier.".into())
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value)
        .map_err(|_| "Invalid operation details. Review the form and retry.".into())
}

#[cfg(unix)]
fn launch(target: &std::ffi::OsStr) -> Result<()> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Cannot open the default application. Check your desktop configuration.")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(windows)]
fn launch(target: &std::ffi::OsStr) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let argument: Vec<u16> = target.encode_wide().chain(Some(0)).collect();
    let open: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            open.as_ptr(),
            argument.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as usize <= 32 {
        return Err("Cannot open the default application. Check your Windows defaults.".into());
    }
    Ok(())
}

fn remote_view(remote: github::Remote) -> Value {
    json!({"name":remote.name,"fullName":remote.full_name,"description":remote.description.unwrap_or_default(),"private":remote.private})
}

fn local_action(
    session: &mut Session,
    app: &tauri::AppHandle,
    repo: &Repo,
    action: &str,
    args: &Value,
) -> Result<Value> {
    workspace::verify_local(repo)?;
    let path = &repo.path;
    let account = session
        .db
        .data
        .accounts
        .iter()
        .find(|a| a.id == repo.account_id);
    match action {
        "repoWatch" => {
            let app = app.clone();
            let id = repo.id.clone();
            session.watcher = None;
            session.watcher = Some(git::watch(path, move |result| {
                let _ = app.emit("repo-change", json!({"id":id,"error":result.err()}));
            })?);
        }
        "openFolder" => launch(path.as_os_str())?,
        "repoStatus" => {
            let s = git::snapshot(path)?;
            let author = s.identity.with_account(account);
            return Ok(
                json!({"branch":s.branch.unwrap_or_else(||"Detached HEAD".into()),"unpushed":s.ahead,
                "authorName":author.name,"authorEmail":author.email,
                "changes":s.changes.into_iter().map(|c|json!({"path":c.path,"oldPath":c.original,"index":c.index,"worktree":c.worktree})).collect::<Vec<_>>() }),
            );
        }
        "history" => return Ok(json!(git::snapshot(path)?.history)),
        "branches" => {
            let s = git::snapshot(path)?;
            return Ok(json!(s
                .branches
                .iter()
                .map(|b| json!({"name":b,"current":Some(b)==s.branch.as_ref()}))
                .collect::<Vec<_>>()));
        }
        "branch" => git::branch(
            path,
            field(args, "operation")?,
            field(args, "name")?,
            args["newName"].as_str().unwrap_or(""),
        )?,
        "stage" | "unstage" => {
            let selected: Vec<String> = decode(args["paths"].clone())?;
            if action == "stage" {
                git::stage(path, &selected)?;
            } else {
                git::unstage(path, &selected)?;
            }
            session.review = None;
        }
        "setAuthor" => git::identity(path, field(args, "name")?, field(args, "email")?)?,
        "scan" => {
            session.review = None;
            let review = git::inspect_commit(path)?;
            session.review = Some((repo.id.clone(), review.tree, !review.warnings.is_empty()));
            return Ok(json!(review.warnings));
        }
        "commit" => {
            let (id, tree, sensitive) = session
                .review
                .take()
                .ok_or("Review the staged changes before committing.")?;
            if id != repo.id || (sensitive && args["allowSensitive"] != true) {
                return Err("Confirm the staged file warnings before committing.".into());
            }
            git::commit(
                path,
                field(args, "title")?,
                field(args, "description")?,
                &tree,
                account,
            )?;
        }
        _ => return Err("Unknown repository operation.".into()),
    }
    Ok(Value::Null)
}

impl Service {
    fn with_session<T>(&self, run: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "An operation was interrupted. Restart the application.")?;
        let session = guard.as_mut().map_err(|e| e.clone())?;
        run(session)
    }

    fn handle(&self, app: &tauri::AppHandle, action: &str, args: Value) -> Result<Value> {
        match action {
            "checkForUpdates" => {
                let mut updater = self.updater.try_lock()
                    .map_err(|_| "An update is already in progress.")?;
                // An installer may restart this process. Let any Git transfer finish first.
                let _transfer = self.install_gate.lock().map_err(|_| "Restart the app and retry the update.")?;
                let _session = self.session.lock()
                    .map_err(|_| "Restart the application before installing an update.")?;
                return Ok(json!(updater.check(app)?));
            }
            "authStart" => {
                let expected = args
                    .get("accountId")
                    .map(|_| number(&args, "accountId"))
                    .transpose()?;
                let device = self.github.begin(expected)?;
                // A second browser button remains available if the desktop has no default handler.
                let _ = launch(github::DEVICE_URL.as_ref());
                return Ok(
                    json!({"userCode":device.user_code,"session":device.session.to_string()}),
                );
            }
            "authCancel" => {
                self.github.cancel(number(&args, "session")?)?;
                return Ok(Value::Null);
            }
            "authPoll" => {
                let result = self.github.poll(number(&args, "session")?)?;
                if let Some(account) = result {
                    self.with_session(|s| {
                        s.watcher = None;
                        s.review = None;
                        let id = account.id.to_string();
                        s.db.change(|d| {
                            d.accounts.retain(|a| a.id != id);
                            d.accounts.push(AccountEntry {
                                id: id.clone(),
                                login: account.login,
                            });
                            d.active_account = Some(id);
                        })?;
                        Ok(Value::Null)
                    })?;
                    let _ = self.auto_wake.send(autosync::Wake::Account(account.id.to_string()));
                    return Ok(json!({"done":true}));
                }
                return Ok(json!({"done":false}));
            }
            "openInstall" | "openDevice" => {
                launch(
                    if action == "openDevice" {
                        github::DEVICE_URL
                    } else {
                        github::INSTALL_URL
                    }
                    .as_ref(),
                )?;
                return Ok(Value::Null);
            }
            "pickFolder" => return Ok(json!(rfd::FileDialog::new().pick_folder())),
            _ => (),
        }
        self.with_session(|session| self.workspace_action(session, app, action, args))
    }

    fn workspace_action(
        &self,
        session: &mut Session,
        app: &tauri::AppHandle,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        if session.auto.busy.as_deref().is_some_and(|id| args["id"].as_str() == Some(id))
            && !matches!(action, "repoStatus" | "repoWatch" | "openFolder" | "history" | "branches" | "scan") {
            return Err("This repository is synchronizing. Retry when the check finishes.".into());
        }
        let db = &mut session.db;
        match action {
            "autoSyncStatus" => return Ok(json!(session.auto.visible(&db.data))),
            "setAutoSyncEnabled" => {
                let enabled = args["enabled"].as_bool().ok_or("Choose whether to synchronize automatically.")?;
                if session.auto.set_enabled(db, enabled)? {
                    if enabled { let _ = self.auto_wake.send(autosync::Wake::All); }
                    let _ = app.emit("auto-sync", ());
                }
            }
            "state" => {
                return Ok(
                    json!({"accounts":db.data.accounts,"activeAccount":db.data.active_account,
                    "repos": db.data.repos.iter().filter(|r|Some(&r.account_id)==db.data.active_account.as_ref()).collect::<Vec<_>>(),
                    "theme":db.data.theme,"autoSyncEnabled":db.data.auto_sync_enabled,"gitVersion":git::version().ok(),"version":env!("CARGO_PKG_VERSION")}),
                )
            }
            "theme" => {
                let theme = field(&args, "theme")?;
                if !["system", "light", "dark"].contains(&theme) {
                    return Err("Unknown theme.".into());
                }
                db.change(|d| d.theme = theme.into())?;
            }
            "switchAccount" | "signOut" => {
                let id = field(&args, "accountId")?;
                if !db.data.accounts.iter().any(|a| a.id == id) {
                    return Err("This account is no longer signed in.".into());
                }
                session.watcher = None;
                session.review = None;
                if action == "signOut" {
                    self.github.sign_out(number(&args, "accountId")?)?;
                    db.change(|d| {
                        d.accounts.retain(|a| a.id != id);
                        if d.active_account.as_deref() == Some(id) {
                            d.active_account = d.accounts.first().map(|a| a.id.clone());
                        }
                    })?;
                } else {
                    db.change(|d| d.active_account = Some(id.into()))?;
                }
            }
            "remoteList" => {
                return Ok(json!(self
                    .github
                    .repositories(db.active()?)?
                    .into_iter()
                    .map(remote_view)
                    .collect::<Vec<_>>()))
            }
            "repoCreate" => return Ok(json!(workspace::create(db, &self.github, decode(args)?)?)),
            "repoOpen" => {
                return Ok(json!(workspace::open_repo(
                    db,
                    &self.github,
                    Path::new(field(&args, "path")?)
                )?))
            }
            "repoClone" => {
                return Ok(json!(workspace::clone_repo(
                    db,
                    &self.github,
                    field(&args, "fullName")?,
                    Path::new(field(&args, "parent")?)
                )?))
            }
            "repoWatch" if args["id"] == "" => {
                session.watcher = None;
                session.review = None;
            }
            _ => {
                let mut repo = db.repo(field(&args, "id")?)?;
                match action {
                    "repoConnect" => {
                        return Ok(json!(workspace::connect(
                            db,
                            &self.github,
                            &repo,
                            field(&args, "mode")?,
                            field(&args, "target")?,
                        )?))
                    }
                    "repoRelocate" => {
                        workspace::relocate(db, &mut repo, Path::new(field(&args, "path")?))?;
                        session.watcher = None;
                        session.review = None;
                    }
                    "repoAssociate" => {
                        workspace::reassociate(
                            db,
                            &self.github,
                            &mut repo,
                            number(&args, "accountId")?,
                        )?;
                        session.watcher = None;
                        session.review = None;
                    }
                    "repoInfo" => {
                        return Ok(remote_view(workspace::info(db, &self.github, &mut repo)?))
                    }
                    "repoUpdate" => workspace::update(db, &self.github, &mut repo, decode(args)?)?,
                    "sync" => workspace::sync(
                        db,
                        &self.github,
                        &mut repo,
                        field(&args, "operation")?,
                        args.get("publish").cloned().map(decode).transpose()?,
                    )?,
                    "repoDelete" => {
                        session.watcher = None;
                        session.review = None;
                        workspace::delete(
                            db,
                            &self.github,
                            &mut repo,
                            field(&args, "mode")?,
                            field(&args, "confirmation")?,
                        )?;
                    }
                    _ => return local_action(session, app, &repo, action, &args),
                }
            }
        }
        Ok(Value::Null)
    }
}

#[tauri::command]
async fn dispatch(
    app: tauri::AppHandle,
    service: tauri::State<'_, Arc<Service>>,
    action: String,
    args: Value,
) -> Result<Value> {
    let service = Arc::clone(service.inner());
    tauri::async_runtime::spawn_blocking(move || service.handle(&app, &action, args))
        .await
        .map_err(|_| {
            "The operation was interrupted. Check the repository before retrying.".to_owned()
        })?
}

fn main() {
    #[cfg(target_os = "linux")]
    gtk::glib::set_prgname(Some("repo-uploader-and-manager"));
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let dir: PathBuf = app.path().app_config_dir()?;
            let session = Database::load(&dir).map(|db| Session {
                db,
                auto: autosync::State::default(),
                watcher: None,
                review: None,
            });
            let (wake, requests) = std::sync::mpsc::channel();
            let service = Arc::new(Service {
                github: Github::new(),
                updater: Mutex::default(),
                auto_wake: wake,
                install_gate: Mutex::new(()),
                session: Mutex::new(session),
            });
            app.manage(Arc::clone(&service));
            window::open(app, &dir)?;
            autosync::start(service, app.handle().clone(), requests)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![dispatch])
        .run(tauri::generate_context!())
        .expect("Cannot start the desktop application");
}
