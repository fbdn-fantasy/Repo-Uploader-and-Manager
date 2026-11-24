//! GitHub App Device Flow and repository operations. Call on a blocking worker.
//! The webview receives the user code and public records only.
#[cfg(test)]
pub(crate) mod tests;
mod vault;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use vault::{SystemVault, Vault};

type Result<T> = std::result::Result<T, String>;
pub const CLIENT_ID: &str = "Iv23licw15FcmAEQUGOn";
pub const DEVICE_URL: &str = "https://github.com/login/device";
pub const INSTALL_URL: &str = "https://github.com/apps/repo-uploader-and-manager/installations/new";
const INVALID: &str = "GitHub returned an incomplete or unexpected response. Please try again.";
const NETWORK: &str = "GitHub could not be reached securely. Check the connection and retry. For interrupted changes, check the repository on GitHub before retrying.";
const INSTALL: &str = "The GitHub App has no accessible installation for this account or repository. Open installation settings, select the owner and repository, and request organization owner approval if required.";
const ADMIN: &str = "Repository administration was denied. Check your administrator role, the app's Administration write permission, and organization policies.";
const REAUTH: &str =
    "GitHub authorization has expired or was revoked. Reauthorize this account in Accounts.";
const CANCELLED: &str = "This sign-in attempt was cancelled or replaced.";
const EXPIRED: &str = "The device code expired. Start a new sign-in.";

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Account {
    pub id: u64,
    pub login: String,
}
#[derive(Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Permissions {
    pub admin: bool,
    pub push: bool,
    pub pull: bool,
    // GitHub omits the entire permission object in some responses.
    // A missing object uses Default (false); a supplied object explicitly reports permissions.
    #[serde(skip, default = "present")]
    reported: bool,
}
fn present() -> bool {
    true
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Remote {
    pub id: u64,
    pub full_name: String,
    pub name: String,
    pub description: Option<String>,
    pub private: bool,
    #[serde(default)]
    pub permissions: Permissions,
    pub default_branch: String,
}
#[derive(Serialize)]
pub struct Device {
    pub session: u64,
    pub user_code: String,
    pub expires_in: u64,
    pub interval: u64,
}
#[derive(Deserialize)]
struct DeviceReply {
    device_code: String,
    user_code: String,
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
    verification_uri: String,
}
fn default_interval() -> u64 {
    5
}

struct Pending {
    session: u64,
    device_code: String,
    expected: Option<u64>,
    expires: Instant,
    next: Instant,
    interval: u64,
    in_flight: bool,
}

#[derive(Deserialize, Serialize)]
struct Credentials {
    access: String,
    refresh: String,
    expires: u64,
    refresh_expires: u64,
}

impl Credentials {
    fn from_reply(body: Value) -> Result<Self> {
        #[derive(Deserialize)]
        struct Reply {
            access_token: String,
            refresh_token: String,
            expires_in: u64,
            refresh_token_expires_in: u64,
            token_type: String,
        }
        let r: Reply = serde_json::from_value(body).map_err(|_| "GitHub did not return an expiring token pair. Check the app's user-token expiration setting and reauthorize.")?;
        if !r.token_type.eq_ignore_ascii_case("bearer")
            || r.access_token.is_empty()
            || r.refresh_token.is_empty()
            || r.expires_in == 0
            || r.refresh_token_expires_in == 0
        {
            return Err(INVALID.into());
        }
        let now = unix_time();
        Ok(Self {
            access: r.access_token,
            refresh: r.refresh_token,
            expires: now.saturating_add(r.expires_in),
            refresh_expires: now.saturating_add(r.refresh_token_expires_in),
        })
    }
}

#[derive(Deserialize)]
struct Installation {
    id: u64,
    account: Option<InstallationOwner>,
    suspended_at: Option<String>,
}

#[derive(Deserialize)]
struct InstallationOwner {
    login: Option<String>,
}

enum RequestKind {
    OAuth,
    CreateRepository,
    Api,
}

pub struct Github {
    agent: ureq::Agent,
    vault: Box<dyn Vault>,
    api_base: String,
    oauth_base: String,
    generation: AtomicU64,
    pending: Mutex<Option<Pending>>,
    // Serialize read/rotate/store and sign-out so a consumed refresh token is never reused.
    credentials: Mutex<()>,
    rate_limits: Mutex<HashMap<Option<u64>, u64>>,
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| "The account service was interrupted. Restart the application.".into())
}
fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn decode<T: DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|_| INVALID.into())
}
fn after(seconds: u64) -> Result<Instant> {
    Instant::now()
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(|| INVALID.into())
}

fn valid_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn split_name(full: &str) -> Result<(&str, &str)> {
    full.split_once('/')
        .filter(|(owner, name)| valid_part(owner) && valid_part(name))
        .ok_or_else(|| "Expected a GitHub owner and repository name.".into())
}

fn auth_error(code: &str) -> String {
    match code {
        "device_flow_disabled" => "Device Flow is disabled. The GitHub App owner must enable Device Flow in the app registration.",
        "access_denied" => "GitHub authorization was declined. Start a new sign-in when ready.",
        "expired_token" | "token_expired" => EXPIRED,
        "incorrect_device_code" => "GitHub rejected the device code. Start a new sign-in.",
        "Not Found" | "incorrect_client_credentials" => "GitHub did not recognize the registered public client ID. Check the application registration.",
        "bad_refresh_token" | "invalid_grant" | "invalid_token" => REAUTH,
        _ => "GitHub rejected the authorization request. Check the GitHub App registration and reauthorize.",
    }.into()
}

impl Github {
    pub fn new() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .user_agent("Repo-Uploader-and-Manager/1.0")
                .max_redirects(0)
                .timeout_global(Some(Duration::from_secs(35)))
                .http_status_as_error(false)
                .build()
                .into(),
            vault: Box::new(SystemVault),
            api_base: "https://api.github.com".into(),
            oauth_base: "https://github.com".into(),
            generation: AtomicU64::new(0),
            pending: Mutex::new(None),
            credentials: Mutex::new(()),
            rate_limits: Mutex::new(HashMap::new()),
        }
    }

    fn rate_check(&self, account: Option<u64>) -> Result<()> {
        let until = lock(&self.rate_limits)?.get(&account).copied().unwrap_or(0);
        let wait = until.saturating_sub(unix_time());
        if wait > 0 {
            Err(format!(
                "GitHub's rate limit is active. Retry in {wait} seconds."
            ))
        } else {
            Ok(())
        }
    }

    // Never surface HTTP error text, response bodies, request URLs, or TLS diagnostics.
    fn response(
        &self,
        account: Option<u64>,
        response: ureq::http::Response<ureq::Body>,
        kind: RequestKind,
    ) -> Result<Value> {
        let (parts, mut body) = response.into_parts();
        let header = |name| parts.headers.get(name).and_then(|v| v.to_str().ok());
        let retry = header("retry-after").and_then(|n| n.parse::<u64>().ok());
        let remaining = header("x-ratelimit-remaining");
        let reset = header("x-ratelimit-reset").and_then(|n| n.parse::<u64>().ok());
        let status = parts.status.as_u16();
        let value: Value = if status == 204 {
            Value::Null
        } else {
            body.with_config()
                .limit(16 * 1024 * 1024)
                .read_json()
                .unwrap_or(Value::Null)
        };
        let throttled = status == 429
            || status == 403
                && (retry.is_some()
                    || remaining == Some("0")
                    || value["message"]
                        .as_str()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains("rate limit"));
        if throttled || remaining == Some("0") {
            let now = unix_time();
            let until = reset
                .unwrap_or(0)
                .max(now.saturating_add(retry.unwrap_or(if throttled { 60 } else { 1 }).max(1)));
            {
                let mut limits = lock(&self.rate_limits)?;
                let saved = limits.entry(account).or_default();
                *saved = (*saved).max(until);
            }
            if throttled {
                return Err(format!(
                    "GitHub's rate limit is active. Retry in {} seconds.",
                    until.saturating_sub(now)
                ));
            }
        }
        if matches!(kind, RequestKind::OAuth) && value["error"].is_string() {
            return Ok(value);
        }
        if matches!(kind, RequestKind::CreateRepository) {
            match status {
                403 | 404 => return Err(format!(
                    "GitHub denied repository creation on your personal account (HTTP {status}). In Manage GitHub App access, check that this account granted Repository creation or Administration write access."
                )),
                422 => {
                    let taken = value["errors"].as_array().is_some_and(|errors| errors.iter().any(|e| {
                        e["field"] == "name" && (e["code"] == "already_exists" ||
                            e["message"].as_str().is_some_and(|m| m.to_ascii_lowercase().contains("already exists")))
                    }));
                    return Err(if taken {
                        "That repository name already exists on GitHub. Choose the existing private repository to connect it, or use another name."
                    } else {
                        "GitHub declined the repository details (HTTP 422). Check the name and whether a repository with that name already exists."
                    }.into());
                }
                _ => {}
            }
        }
        match status {
            200..=299 if status == 204 || !value.is_null() => Ok(value),
            200..=299 => Err(INVALID.into()),
            301 | 302 | 307 | 308 => Err("GitHub reports the repository has moved. Check its new name and local association.".into()),
            401 => Err(REAUTH.into()),
            403 => Err("GitHub denied permission. Check the installation's granted permissions, your repository role, and any organization approval or SSO requirements.".into()),
            404 => Err(format!("GitHub could not find this resource for your account. {INSTALL}")),
            409 => Err("GitHub reported a conflicting change. Refresh the repository and retry.".into()),
            422 => Err("GitHub rejected the repository details. Check for a duplicate or invalid name and organization restrictions.".into()),
            500..=599 => Err("GitHub is temporarily unavailable. Check the remote state before retrying a change.".into()),
            _ => Err(format!("GitHub rejected the request (HTTP {status}).")),
        }
    }

    fn form(&self, path: &str, fields: &[(&str, &str)]) -> Result<Value> {
        self.rate_check(None)?;
        let response = self
            .agent
            .post(format!("{}{path}", self.oauth_base))
            .header("accept", "application/json")
            .send_form(fields.iter().copied())
            .map_err(|_| NETWORK)?;
        self.response(None, response, RequestKind::OAuth)
    }

    pub fn begin(&self, expected: Option<u64>) -> Result<Device> {
        let session = {
            let mut pending = lock(&self.pending)?;
            *pending = None;
            self.generation.fetch_add(1, Ordering::SeqCst) + 1
        };
        let value = self.form(
            "/login/device/code",
            &[("client_id", CLIENT_ID), ("scope", "offline_access")],
        )?;
        if let Some(code) = value["error"].as_str() {
            return Err(auth_error(code));
        }
        let r: DeviceReply = decode(value)?;
        if r.device_code.is_empty()
            || r.user_code.is_empty()
            || r.expires_in == 0
            || r.verification_uri != DEVICE_URL
        {
            return Err(INVALID.into());
        }
        let interval = r.interval.max(1);
        let mut pending = lock(&self.pending)?;
        if self.generation.load(Ordering::SeqCst) != session {
            return Err(CANCELLED.into());
        }
        *pending = Some(Pending {
            session,
            expected,
            device_code: r.device_code,
            expires: after(r.expires_in)?,
            next: after(interval)?,
            interval,
            in_flight: false,
        });
        Ok(Device {
            session,
            user_code: r.user_code,
            expires_in: r.expires_in,
            interval,
        })
    }

    pub fn cancel(&self, session: u64) -> Result<()> {
        let mut state = lock(&self.pending)?;
        if state.as_ref().is_some_and(|p| p.session == session) {
            *state = None;
        }
        Ok(())
    }

    pub fn poll(&self, session: u64) -> Result<Option<Account>> {
        let (device_code, expected) = {
            let mut state = lock(&self.pending)?;
            let p = state
                .as_mut()
                .filter(|p| p.session == session)
                .ok_or(CANCELLED)?;
            let now = Instant::now();
            if now >= p.expires {
                *state = None;
                return Err(EXPIRED.into());
            }
            if p.in_flight || now < p.next {
                return Ok(None);
            }
            p.in_flight = true;
            (p.device_code.clone(), p.expected)
        };
        let operation = self.poll_once(&device_code, expected);
        let mut state = lock(&self.pending)?;
        let p = state
            .as_mut()
            .filter(|p| p.session == session)
            .ok_or(CANCELLED)?;
        p.in_flight = false;
        match operation {
            Ok(Poll::Wait(interval)) => {
                if let Some(interval) = interval {
                    p.interval = p.interval.saturating_add(5).max(interval);
                }
                p.next = after(p.interval)?;
                Ok(None)
            }
            Ok(Poll::Authorized(account, credentials)) => {
                // A completed, cancelled request may never write the account's store item.
                let _lock = lock(&self.credentials)?;
                *state = None;
                self.save(account.id, &credentials)?;
                Ok(Some(account))
            }
            Err(error) => {
                *state = None;
                Err(error)
            }
        }
    }

    fn poll_once(&self, code: &str, expected: Option<u64>) -> Result<Poll> {
        let value = self.form(
            "/login/oauth/access_token",
            &[
                ("client_id", CLIENT_ID),
                ("device_code", code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ],
        )?;
        if let Some(error) = value["error"].as_str() {
            return match error {
                "authorization_pending" => Ok(Poll::Wait(None)),
                "slow_down" => Ok(Poll::Wait(Some(value["interval"].as_u64().unwrap_or(0)))),
                other => Err(auth_error(other)),
            };
        }
        let pair = Credentials::from_reply(value)?;
        let user: Account = decode(self.request(None, &pair.access, "GET", "/user", None)?)?;
        if expected.is_some_and(|id| id != user.id) {
            return Err("The browser authorized a different GitHub account. Select the original account in GitHub and reauthorize.".into());
        }
        Ok(Poll::Authorized(user, pair))
    }

    fn save(&self, account: u64, pair: &Credentials) -> Result<()> {
        self.vault
            .save(account, &serde_json::to_vec(pair).map_err(|_| INVALID)?)
    }

    pub fn token(&self, account: u64) -> Result<String> {
        let _lock = lock(&self.credentials)?;
        let bytes = self.vault.load(account)?;
        let pair: Credentials = serde_json::from_slice(&bytes)
            .map_err(|_| "The saved credential is unreadable. Reauthorize this account.")?;
        let now = unix_time();
        if now.saturating_add(90) < pair.expires {
            return Ok(pair.access);
        }
        if now >= pair.refresh_expires {
            return Err(REAUTH.into());
        }
        let value = self.form(
            "/login/oauth/access_token",
            &[
                ("client_id", CLIENT_ID),
                ("grant_type", "refresh_token"),
                ("refresh_token", &pair.refresh),
            ],
        )?;
        if let Some(code) = value["error"].as_str() {
            return Err(auth_error(code));
        }
        let rotated = Credentials::from_reply(value)?;
        self.save(account, &rotated).map_err(|e| format!("GitHub rotated the credential but it could not be saved. Reauthorize this account. {e}"))?;
        Ok(rotated.access)
    }

    pub fn sign_out(&self, account: u64) -> Result<()> {
        let _lock = lock(&self.credentials)?;
        self.vault.remove(account)
    }

    fn request(
        &self,
        account: Option<u64>,
        token: &str,
        method: &str,
        path: &str,
        data: Option<Value>,
    ) -> Result<Value> {
        self.rate_check(account)?;
        let url = format!("{}{path}", self.api_base);
        let mut builder = ureq::http::Request::builder()
            .uri(url)
            .method(method)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2026-03-10");
        let bytes = match data {
            Some(v) => {
                builder = builder.header("Content-Type", "application/json");
                serde_json::to_vec(&v).map_err(|_| INVALID)?
            }
            None => Vec::new(),
        };
        let response = self
            .agent
            .run(builder.body(bytes.as_slice()).map_err(|_| INVALID)?)
            .map_err(|_| NETWORK)?;
        self.response(
            account,
            response,
            if method == "POST" && path == "/user/repos" {
                RequestKind::CreateRepository
            } else {
                RequestKind::Api
            },
        )
    }

    fn page_list<T: DeserializeOwned>(
        &self,
        account: u64,
        token: &str,
        path: &str,
        key: &str,
    ) -> Result<Vec<T>> {
        let mut result = Vec::new();
        for page in 1u64.. {
            let mut body = self.request(
                Some(account),
                token,
                "GET",
                &format!("{path}?per_page=100&page={page}"),
                None,
            )?;
            let rows: Vec<T> = decode(body[key].take())?;
            let more = rows.len() == 100;
            result.extend(rows);
            if !more {
                break;
            }
        }
        Ok(result)
    }

    fn installations(&self, account: u64, token: &str) -> Result<Vec<Installation>> {
        let mut list: Vec<Installation> =
            self.page_list(account, token, "/user/installations", "installations")?;
        list.retain(|i| i.suspended_at.is_none());
        if list.is_empty() {
            return Err(INSTALL.into());
        }
        Ok(list)
    }

    fn installed_repos(&self, account: u64, token: &str, id: u64) -> Result<Vec<Remote>> {
        self.page_list(
            account,
            token,
            &format!("/user/installations/{id}/repositories"),
            "repositories",
        )
    }

    pub fn repositories(&self, account: u64) -> Result<Vec<Remote>> {
        let token = self.token(account)?;
        let mut rows = Vec::new();
        for i in self.installations(account, &token)? {
            rows.extend(self.installed_repos(account, &token, i.id)?);
        }
        rows.sort_by_cached_key(|r| r.full_name.to_ascii_lowercase());
        rows.dedup_by_key(|r| r.id);
        Ok(rows)
    }

    fn accessible(&self, account: u64, token: &str, full: &str) -> Result<Remote> {
        let (owner, _) = split_name(full)?;
        let installs = self.installations(account, token)?;
        let install = installs
            .iter()
            .find(|i| {
                i.account
                    .as_ref()
                    .and_then(|a| a.login.as_ref())
                    .is_some_and(|login| login.eq_ignore_ascii_case(owner))
            })
            .ok_or(INSTALL)?;
        let rows = self.installed_repos(account, token, install.id)?;
        rows.into_iter()
            .find(|r| r.full_name.eq_ignore_ascii_case(full))
            .ok_or_else(|| INSTALL.into())
    }

    pub fn repository(&self, account: u64, full: &str) -> Result<Remote> {
        let token = self.token(account)?;
        let allowed = self.accessible(account, &token, full)?;
        self.details(account, &token, full, allowed)
    }

    /// Background synchronization checks only the already-saved destination.
    /// Auth uses its associated account even when a different account is active.
    pub fn sync_repository_id(
        &self,
        account: u64,
        token: &str,
        full: &str,
        expected: Option<u64>,
    ) -> Result<u64> {
        let (_, name) = split_name(full)?;
        let remote: Remote =
            decode(self.request(Some(account), token, "GET", &format!("/repos/{full}"), None)?)?;
        if remote.id == 0
            || !remote.full_name.eq_ignore_ascii_case(full)
            || !remote.name.eq_ignore_ascii_case(name)
            || expected.is_some_and(|id| id != remote.id)
        {
            return Err("The GitHub repository at this name has changed. Verify the repository association before synchronizing.".into());
        }
        Ok(remote.id)
    }

    fn details(&self, account: u64, token: &str, full: &str, allowed: Remote) -> Result<Remote> {
        let mut current: Remote =
            decode(self.request(Some(account), token, "GET", &format!("/repos/{full}"), None)?)?;
        if current.id != allowed.id {
            return Err(
                "The repository identity changed during the operation. Refresh before continuing."
                    .into(),
            );
        }
        if !current.permissions.reported {
            current.permissions = allowed.permissions;
        }
        Ok(current)
    }

    pub fn create(&self, account: u64, name: &str, description: &str) -> Result<Remote> {
        if !valid_part(name) {
            return Err("Enter a valid GitHub repository name.".into());
        }
        let token = self.token(account)?;
        let repo: Remote = decode(self.request(
            Some(account),
            &token,
            "POST",
            "/user/repos",
            Some(
                json!({"name":name, "description":description, "private":true, "auto_init":false}),
            ),
        )?)?;
        if !repo.private {
            return Err("GitHub did not confirm a private repository. Inspect the remote before continuing.".into());
        }
        let (_, returned_name) = split_name(&repo.full_name)?;
        if !returned_name.eq_ignore_ascii_case(name) || !repo.name.eq_ignore_ascii_case(name) {
            return Err("GitHub returned a different repository name. Check your repositories before retrying.".into());
        }
        Ok(repo)
    }

    fn check_admin(&self, account: u64, token: &str, full: &str, expected_id: u64) -> Result<()> {
        let mut allowed = self.accessible(account, token, full)?;
        if allowed.id != expected_id {
            return Err("This repository name now refers to a different GitHub repository. The requested change was cancelled.".into());
        }
        if !allowed.permissions.reported {
            allowed = self.details(account, token, full, allowed)?;
        }
        if !allowed.permissions.admin {
            return Err(ADMIN.into());
        }
        Ok(())
    }

    pub fn update(
        &self,
        account: u64,
        full: &str,
        name: Option<&str>,
        description: Option<&str>,
        private: Option<bool>,
        expected_id: u64,
    ) -> Result<Remote> {
        if name.is_some_and(|n| !valid_part(n)) {
            return Err("Enter a valid GitHub repository name.".into());
        }
        let token = self.token(account)?;
        self.check_admin(account, &token, full, expected_id)?;
        let mut data = json!({});
        if let Some(value) = name {
            data["name"] = value.into();
        }
        if let Some(value) = description {
            data["description"] = value.into();
        }
        if let Some(value) = private {
            data["private"] = value.into();
        }
        let updated: Remote = decode(self.request(
            Some(account),
            &token,
            "PATCH",
            &format!("/repos/{full}"),
            Some(data),
        )?)?;
        if updated.id != expected_id || private.is_some_and(|p| p != updated.private) {
            return Err("GitHub did not confirm the requested repository settings. Refresh before retrying.".into());
        }
        Ok(updated)
    }

    pub fn delete(&self, account: u64, full: &str, expected_id: u64) -> Result<()> {
        let token = self.token(account)?;
        self.check_admin(account, &token, full, expected_id)?;
        self.request(
            Some(account),
            &token,
            "DELETE",
            &format!("/repos/{full}"),
            None,
        )?;
        Ok(())
    }
}
enum Poll {
    Wait(Option<u64>),
    Authorized(Account, Credentials),
}
