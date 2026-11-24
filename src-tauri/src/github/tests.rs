//! Loopback fixtures exercise the HTTP wire format and use an in-memory vault.
use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    thread::{self, JoinHandle},
};

#[derive(Default)]
struct MemoryVault(Mutex<HashMap<u64, Vec<u8>>>);
impl Vault for MemoryVault {
    fn load(&self, id: u64) -> Result<Vec<u8>> {
        lock(&self.0)?
            .get(&id)
            .cloned()
            .ok_or_else(|| vault::MISSING.into())
    }
    fn save(&self, id: u64, value: &[u8]) -> Result<()> {
        lock(&self.0)?.insert(id, value.to_vec());
        Ok(())
    }
    fn remove(&self, id: u64) -> Result<()> {
        lock(&self.0)?.remove(&id);
        Ok(())
    }
}

pub(crate) struct Reply {
    pub(crate) method: &'static str,
    pub(crate) path: &'static str,
    pub(crate) status: u16,
    pub(crate) json: Value,
    pub(crate) retry: bool,
}
pub(crate) fn reply(method: &'static str, path: &'static str, json: Value) -> Reply {
    Reply {
        method,
        path,
        status: 200,
        json,
        retry: false,
    }
}
pub(crate) struct Request {
    pub(crate) headers: String,
    pub(crate) body: String,
}
pub(crate) fn service(script: Vec<Reply>) -> (Github, JoinHandle<Vec<Request>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for next in script {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut headers = String::new();
            loop {
                let len = reader.read_line(&mut headers).unwrap();
                assert!(len > 0);
                if headers.ends_with("\r\n\r\n") {
                    break;
                }
            }
            assert!(headers.starts_with(&format!("{} {} HTTP/", next.method, next.path)));
            let size = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            let mut payload = vec![0; size];
            reader.read_exact(&mut payload).unwrap();
            requests.push(Request {
                headers,
                body: String::from_utf8(payload).unwrap(),
            });
            let body = if next.status == 204 {
                String::new()
            } else {
                next.json.to_string()
            };
            let rate = if next.retry {
                "Retry-After: 63\r\n"
            } else {
                ""
            };
            write!(stream, "HTTP/1.1 {} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{rate}\r\n{body}",next.status,body.len()).unwrap();
        }
        requests
    });
    let mut github = Github::new();
    github.agent = ureq::Agent::config_builder()
        .proxy(None)
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into();
    github.oauth_base = url.clone();
    github.api_base = url;
    github.vault = Box::<MemoryVault>::default();
    (github, server)
}

fn device() -> Reply {
    reply(
        "POST",
        "/login/device/code",
        json!({"device_code":"test-device","user_code":"ABCD-EFGH","expires_in":900,"interval":5,"verification_uri":DEVICE_URL}),
    )
}
fn oauth(value: Value) -> Reply {
    reply("POST", "/login/oauth/access_token", value)
}
fn pair(a: &str, r: &str) -> Value {
    json!({"token_type":"bearer","access_token":a,"expires_in":28800,"refresh_token":r,"refresh_token_expires_in":15897600})
}
fn ready(g: &Github) {
    g.pending.lock().unwrap().as_mut().unwrap().next = Instant::now();
}
pub(crate) fn login(g: &Github, id: u64, key: &str, expiry: u64) {
    let pair = Credentials {
        access: key.into(),
        refresh: format!("{key}-next"),
        expires: expiry,
        refresh_expires: unix_time() + 1200,
    };
    g.save(id, &pair).unwrap();
}
const INSTALLATIONS_PATH: &str = "/user/installations?per_page=100&page=1";
const REPOS_PATH: &str = "/user/installations/10/repositories?per_page=100&page=1";
pub(crate) fn installation() -> Reply {
    reply(
        "GET",
        INSTALLATIONS_PATH,
        json!({"installations":[{"id":10,"account":{"id":1,"login":"fixture"}}]}),
    )
}
pub(crate) fn remote(name: &str, private: bool) -> Value {
    json!({"id":100,"name":name,"full_name":format!("fixture/{name}"),"description":"test","private":private,
        "permissions":{"admin":true,"pull":true,"push":true},"default_branch":"main"})
}
pub(crate) fn listing(name: &str) -> Reply {
    reply(
        "GET",
        REPOS_PATH,
        json!({"repositories":[remote(name,true)]}),
    )
}

#[test]
fn device_flow_respects_delay_slowdown_and_account_identity() {
    let (g, server) = service(vec![
        device(),
        oauth(json!({"error":"authorization_pending"})),
        oauth(json!({"error":"slow_down","interval":12})),
        oauth(pair("fixture-access", "fixture-refresh")),
        reply("GET", "/user", json!({"id":7,"login":"signed-in"})),
    ]);
    let d = g.begin(Some(7)).unwrap();
    assert_eq!(d.user_code, "ABCD-EFGH");
    assert!(g.poll(d.session).unwrap().is_none()); // No premature HTTP request
    ready(&g);
    assert!(g.poll(d.session).unwrap().is_none());
    ready(&g);
    assert!(g.poll(d.session).unwrap().is_none());
    assert_eq!(g.pending.lock().unwrap().as_ref().unwrap().interval, 12);
    assert!(g.poll(d.session).unwrap().is_none());
    ready(&g);
    assert_eq!(g.poll(d.session).unwrap().unwrap().id, 7);
    assert_eq!(g.token(7).unwrap(), "fixture-access");
    let requests = server.join().unwrap();
    assert!(requests[0].body.contains(CLIENT_ID));
    assert!(requests[1].body.contains("device_code=test-device"));
    assert!(requests[4]
        .headers
        .to_lowercase()
        .contains("authorization: bearer fixture-access"));
    assert!(!requests.iter().any(|r| r.body.contains("client_secret")));
}

#[test]
fn cancel_and_expire_do_not_poll_or_store() {
    let (g, server) = service(vec![device(), device()]);
    let first = g.begin(None).unwrap();
    g.cancel(first.session).unwrap();
    assert_eq!(g.poll(first.session).err().unwrap(), CANCELLED);
    let second = g.begin(None).unwrap();
    g.pending.lock().unwrap().as_mut().unwrap().expires = Instant::now();
    assert_eq!(g.poll(second.session).err().unwrap(), EXPIRED);
    assert!(g.token(1).is_err());
    server.join().unwrap();
}

#[test]
fn disabled_denied_and_wrong_account_never_replace_credentials() {
    let (g, server) = service(vec![
        reply(
            "POST",
            "/login/device/code",
            json!({"error":"device_flow_disabled"}),
        ),
        device(),
        oauth(json!({"error":"access_denied"})),
        device(),
        oauth(pair("incoming", "incoming-refresh")),
        reply("GET", "/user", json!({"id":2,"login":"other"})),
    ]);
    login(&g, 1, "existing", unix_time() + 300);
    assert!(g.begin(None).err().unwrap().contains("disabled"));
    let first = g.begin(None).unwrap();
    ready(&g);
    assert!(g.poll(first.session).err().unwrap().contains("declined"));
    let second = g.begin(Some(1)).unwrap();
    ready(&g);
    assert!(g
        .poll(second.session)
        .err()
        .unwrap()
        .contains("different GitHub account"));
    assert_eq!(g.token(1).unwrap(), "existing");
    assert!(g.token(2).is_err());
    server.join().unwrap();
}

#[test]
fn refresh_rotates_pair_once_and_isolates_accounts() {
    let (g, server) = service(vec![
        oauth(pair("replacement", "replacement-next")),
        oauth(json!({"error":"bad_refresh_token"})),
    ]);
    login(&g, 1, "old", unix_time());
    login(&g, 2, "second", unix_time() + 800);
    thread::scope(|s| {
        let a = s.spawn(|| g.token(1).unwrap());
        let b = s.spawn(|| g.token(1).unwrap());
        assert_eq!(a.join().unwrap(), "replacement");
        assert_eq!(b.join().unwrap(), "replacement");
    });
    let saved: Credentials = serde_json::from_slice(&g.vault.load(1).unwrap()).unwrap();
    assert_eq!(saved.refresh, "replacement-next");
    login(&g, 3, "revoked", unix_time());
    assert_eq!(g.token(3).unwrap_err(), REAUTH);
    g.sign_out(1).unwrap();
    assert!(g.token(1).is_err());
    assert_eq!(g.token(2).unwrap(), "second");
    let requests = server.join().unwrap();
    assert!(requests[0].body.contains("grant_type=refresh_token"));
    assert!(requests[0].body.contains("refresh_token=old-next"));
    assert!(!requests[0].body.contains("client_secret"));
}

#[test]
fn repository_requests_are_scoped_private_and_use_documented_verbs() {
    let (g, server) = service(vec![
        installation(),
        listing("alpha"),
        reply("POST", "/user/repos", remote("alpha", true)),
        installation(),
        listing("alpha"),
        reply("GET", "/repos/fixture/alpha", remote("alpha", true)),
        installation(),
        listing("alpha"),
        reply("PATCH", "/repos/fixture/alpha", remote("beta", false)),
        installation(),
        listing("beta"),
        Reply {
            status: 204,
            ..reply("DELETE", "/repos/fixture/beta", Value::Null)
        },
    ]);
    login(&g, 1, "one", unix_time() + 1000);
    assert_eq!(g.repositories(1).unwrap()[0].id, 100);
    assert!(g.create(1, "alpha", "test").unwrap().private);
    assert_eq!(g.repository(1, "fixture/alpha").unwrap().name, "alpha");
    assert!(
        !g.update(
            1,
            "fixture/alpha",
            Some("beta"),
            Some("edit"),
            Some(false),
            100
        )
        .unwrap()
        .private
    );
    g.delete(1, "fixture/beta", 100).unwrap();
    let requests = server.join().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&requests[2].body).unwrap(),
        json!({"name":"alpha","private":true,"description":"test","auto_init":false})
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[8].body).unwrap(),
        json!({"name":"beta","private":false,"description":"edit"})
    );
    assert!(requests.iter().all(|r| r
        .headers
        .to_lowercase()
        .contains("authorization: bearer one")));
}

#[test]
fn missing_installation_revoked_access_duplicate_and_rate_limits_are_actionable() {
    let (g, server) = service(vec![
        reply("GET", INSTALLATIONS_PATH, json!({"installations":[]})),
        installation(),
        reply("GET", REPOS_PATH, json!({"repositories":[]})),
        Reply {
            status: 401,
            ..installation()
        },
        Reply {
            status: 422,
            ..reply(
                "POST",
                "/user/repos",
                json!({"errors":[{"field":"name","code":"already_exists"}],"message":"private-response"}),
            )
        },
        Reply {
            status: 429,
            retry: true,
            ..installation()
        },
    ]);
    login(&g, 1, "one", unix_time() + 1000);
    assert!(g.repositories(1).err().unwrap().contains("installation"));
    assert!(g
        .repository(1, "fixture/missing")
        .err()
        .unwrap()
        .contains("installation"));
    assert_eq!(g.repositories(1).err().unwrap(), REAUTH);
    let error = g.create(1, "duplicate", "").err().unwrap();
    assert!(error.contains("already exists"));
    assert!(!error.contains("private-response"));
    assert!(g.repositories(1).err().unwrap().contains("63 seconds"));
    assert!(g.repositories(1).err().unwrap().contains("rate limit"));
    server.join().unwrap();
}

#[test]
#[ignore = "Initiates one real GitHub Device Flow and cancels it locally. Requires network access."]
fn live_initiation_and_cancellation() {
    let g = Github::new();
    let device = g.begin(None).unwrap();
    assert!(device.interval > 0 && !device.user_code.is_empty());
    g.cancel(device.session).unwrap();
    assert_eq!(g.poll(device.session).err().unwrap(), CANCELLED);
}

#[test]
fn replacement_repo_and_missing_admin_abort_before_mutation() {
    let mut read_only = remote("alpha", true);
    read_only["permissions"]["admin"] = false.into();
    let (g, server) = service(vec![
        installation(),
        listing("alpha"),
        installation(),
        listing("alpha"),
        installation(),
        reply("GET", REPOS_PATH, json!({"repositories":[read_only]})),
    ]);
    login(&g, 1, "one", unix_time() + 1000);
    assert!(g
        .delete(1, "fixture/alpha", 87)
        .err()
        .unwrap()
        .contains("different GitHub repository"));
    assert!(g
        .update(1, "fixture/alpha", None, None, Some(false), 87)
        .err()
        .unwrap()
        .contains("different GitHub repository"));
    assert_eq!(g.delete(1, "fixture/alpha", 100).err().unwrap(), ADMIN);
    server.join().unwrap();
}

#[test]
fn expired_refresh_and_incorrect_device_code_require_authorization() {
    let (g, server) = service(vec![
        device(),
        oauth(json!({"error":"incorrect_device_code"})),
    ]);
    g.save(
        1,
        &Credentials {
            access: "fixture".into(),
            refresh: "fixture-next".into(),
            expires: 0,
            refresh_expires: 0,
        },
    )
    .unwrap();
    assert_eq!(g.token(1).unwrap_err(), REAUTH);
    let d = g.begin(None).unwrap();
    ready(&g);
    assert!(g
        .poll(d.session)
        .err()
        .unwrap()
        .contains("rejected the device code"));
    server.join().unwrap();
}

#[test]
fn optional_permissions_are_resolved_against_matching_repository_id() {
    let known = remote("alpha", true);
    let mut sparse = known.clone();
    sparse.as_object_mut().unwrap().remove("permissions");
    let (g, server) = service(vec![
        installation(),
        listing("alpha"),
        reply("GET", "/repos/fixture/alpha", sparse.clone()),
        installation(),
        reply("GET", REPOS_PATH, json!({"repositories":[sparse.clone()]})),
        reply("GET", "/repos/fixture/alpha", known.clone()),
        reply("PATCH", "/repos/fixture/alpha", known),
        Reply {
            status: 403,
            ..reply("POST", "/user/repos", json!({"message":"private-response"}))
        },
    ]);
    login(&g, 1, "one", unix_time() + 1000);
    assert!(g.repository(1, "fixture/alpha").unwrap().permissions.admin);
    assert!(g
        .update(1, "fixture/alpha", None, Some("test"), None, 100)
        .is_ok());
    assert!(g
        .create(1, "alpha", "")
        .err()
        .unwrap()
        .contains("Repository creation"));
    server.join().unwrap();
}

#[test]
fn background_lookup_pins_only_exact_matching_names_for_legacy_checkouts() {
    let (api, server) = service(vec![
        reply("GET", "/repos/fixture/sample", remote("different", true)),
        reply("GET", "/repos/fixture/sample", remote("sample", true)),
    ]);
    assert!(api
        .sync_repository_id(1, "fixture", "../outside", None)
        .is_err());
    assert!(api
        .sync_repository_id(1, "fixture", "fixture/sample", None)
        .is_err());
    assert_eq!(
        api.sync_repository_id(1, "fixture", "fixture/sample", None)
            .unwrap(),
        100
    );
    assert_eq!(server.join().unwrap().len(), 2);
}
