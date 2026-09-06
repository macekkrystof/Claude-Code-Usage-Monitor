//! Standalone ChatGPT login. No CLI processes or CLI credential writes.
//! Protocol reference: openai/codex ac192cd7937b0d73edc6dffe009940ae53782dd4,
//! codex-rs/login/src/{server.rs,auth/manager.rs}. See README for compatibility.
use crate::accounts::{now_unix, Account};
use crate::poller::PollError;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, HANDLE, HLOCAL, WAIT_ABANDONED, WAIT_OBJECT_0,
};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

const ISSUER: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TIMEOUT: Duration = Duration::from_secs(300);

// Intentionally no Debug: never accidentally print credentials.
#[derive(Clone, Serialize, Deserialize)]
pub struct Tokens {
    access_token: String,
    refresh_token: String,
    account_id: String,
    expires_at: u64,
    #[serde(default)]
    invalid: bool,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<u64>,
}

pub struct LoginResult {
    pub account: Account,
    pub tokens: Tokens,
}

pub struct LoginTask {
    pub receiver: mpsc::Receiver<Result<LoginResult, String>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for LoginTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub fn start(account: Account) -> LoginTask {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(login(account, &worker_cancel));
    });
    LoginTask { receiver, cancel }
}

fn random_code() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn authorize_url(redirect: &str, state: &str, verifier: &str) -> url::Url {
    let mut url = url::Url::parse(&format!("{ISSUER}/oauth/authorize")).unwrap();
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect),
        (
            "scope",
            "openid profile email offline_access api.connectors.read api.connectors.invoke",
        ),
        (
            "code_challenge",
            &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        ),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("prompt", "login"),
    ]);
    url
}

fn open_browser(url: &str) -> Result<(), String> {
    use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let verb = crate::native_interop::wide_str("open");
    let target = crate::native_interop::wide_str(url);
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(target.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        Err("Unable to open the sign-in browser".into())
    } else {
        Ok(())
    }
}

fn bind_callback() -> Result<tiny_http::Server, String> {
    [1455, 1457]
        .into_iter()
        .find_map(|port| tiny_http::Server::http(("127.0.0.1", port)).ok())
        .ok_or_else(|| "Sign-in ports are busy. Finish another sign-in and try again".into())
}

fn callback_code(path: &str, state: &str) -> Result<Option<String>, &'static str> {
    let url = url::Url::parse(&format!("http://localhost{path}"))
        .map_err(|_| "Invalid sign-in callback")?;
    if url.path() != "/auth/callback" {
        return Ok(None);
    }
    let values: Vec<_> = url.query_pairs().collect();
    let one = |key: &str| {
        let mut entries = values.iter().filter(|(name, _)| name == key);
        let result = entries.next().map(|(_, value)| value.to_string());
        if entries.next().is_some() {
            None
        } else {
            result
        }
    };
    if one("state").as_deref() != Some(state) {
        return Err("Invalid sign-in state");
    }
    if one("error").is_some() {
        return Err("Sign-in was declined");
    }
    one("code")
        .filter(|code| !code.is_empty())
        .map(Some)
        .ok_or("Missing sign-in code")
}

fn login(account: Account, cancel: &AtomicBool) -> Result<LoginResult, String> {
    let server = bind_callback()?;
    login_at(
        account,
        cancel,
        server,
        &format!("{ISSUER}/oauth/token"),
        TIMEOUT,
        open_browser,
    )
}

fn login_at(
    mut account: Account,
    cancel: &AtomicBool,
    server: tiny_http::Server,
    token_endpoint: &str,
    timeout: Duration,
    open: impl FnOnce(&str) -> Result<(), String>,
) -> Result<LoginResult, String> {
    let port = server
        .server_addr()
        .to_ip()
        .ok_or("Unable to start sign-in listener")?
        .port();
    let redirect = format!("http://localhost:{port}/auth/callback");
    let state = random_code();
    let verifier = random_code();
    open(authorize_url(&redirect, &state, &verifier).as_str())?;
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Sign-in cancelled".into());
        }
        if started.elapsed() >= timeout {
            return Err("Sign-in timed out. Try again".into());
        }
        let Some(request) = server
            .recv_timeout(Duration::from_millis(100))
            .map_err(|_| "Unable to receive sign-in callback")?
        else {
            continue;
        };
        if request.method() != &tiny_http::Method::Get || request.url().len() > 16384 {
            let _ = request.respond(tiny_http::Response::empty(400));
            continue;
        }
        match callback_code(request.url(), &state) {
            Ok(None) => {
                let _ = request.respond(tiny_http::Response::empty(404));
            }
            Err(error) => {
                let _ =
                    request.respond(tiny_http::Response::from_string(error).with_status_code(400));
                if error == "Sign-in was declined" {
                    return Err(error.into());
                }
            }
            Ok(Some(code)) => {
                let result = exchange_code(token_endpoint, &code, &redirect, &verifier)
                    .and_then(|response| credentials_from_response(&mut account, response));
                let message = if result.is_ok() {
                    "Sign-in complete. You can return to Usage Monitor."
                } else {
                    "Sign-in failed. Return to Usage Monitor for details."
                };
                let _ = request.respond(tiny_http::Response::from_string(message));
                if cancel.load(Ordering::Acquire) {
                    return Err("Sign-in cancelled".into());
                }
                return result.map(|tokens| LoginResult { account, tokens });
            }
        }
    }
}

fn agent() -> Result<ureq::Agent, String> {
    let tls =
        native_tls::TlsConnector::new().map_err(|_| "Unable to initialize secure connection")?;
    Ok(ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .redirects(0)
        .tls_connector(Arc::new(tls))
        .build())
}

fn exchange_code(
    endpoint: &str,
    code: &str,
    redirect: &str,
    verifier: &str,
) -> Result<TokenResponse, String> {
    agent()?
        .post(endpoint)
        .send_form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("redirect_uri", redirect),
            ("code_verifier", verifier),
        ])
        .map_err(|_| "Unable to complete sign-in. Try again")?
        .into_json()
        .map_err(|_| "Invalid sign-in response".into())
}

fn claims(jwt: &str) -> Result<serde_json::Value, String> {
    let encoded = jwt.split('.').nth(1).ok_or("Missing account identity")?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "Invalid account identity")?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid account identity".into())
}

fn credentials_from_response(
    account: &mut Account,
    response: TokenResponse,
) -> Result<Tokens, String> {
    // Claims arrive only in the TLS-authenticated token response, never from the callback.
    let identity = claims(
        response
            .id_token
            .as_deref()
            .ok_or("Missing account identity")?,
    )?;
    let auth = &identity["https://api.openai.com/auth"];
    let subject = identity["sub"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Missing account identity")?;
    let workspace = auth["chatgpt_account_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("This account has no Codex workspace")?;
    if !account.subject.is_empty() && (account.subject != subject || account.workspace != workspace)
    {
        return Err("A different account was selected. Use Add account instead".into());
    }
    account.subject = subject.into();
    account.workspace = workspace.into();
    account.identity = identity["email"]
        .as_str()
        .or_else(|| identity["https://api.openai.com/profile"]["email"].as_str())
        .unwrap_or(subject)
        .into();
    if response.access_token.is_empty() {
        return Err("Invalid sign-in response".into());
    }
    let expires_at = expiration(&response.access_token, response.expires_in);
    Ok(Tokens {
        access_token: response.access_token,
        refresh_token: response
            .refresh_token
            .filter(|s| !s.is_empty())
            .ok_or("Missing refresh token")?,
        account_id: workspace.into(),
        expires_at,
        invalid: false,
    })
}

fn expiration(token: &str, expires_in: Option<u64>) -> u64 {
    claims(token)
        .ok()
        .and_then(|c| c["exp"].as_u64())
        .unwrap_or_else(|| now_unix().saturating_add(expires_in.unwrap_or(3600)))
}

struct VaultLock(HANDLE);
impl VaultLock {
    fn acquire(id: &str) -> Result<Self, String> {
        let name = crate::native_interop::wide_str(&format!("Local\\UsageMonitorAccount_{id}"));
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }
            .map_err(|_| "Unable to lock account credentials")?;
        let result = unsafe { WaitForSingleObject(handle, 35000) };
        if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err("Account credentials are busy. Try again".into());
        }
        Ok(Self(handle))
    }
}
impl Drop for VaultLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

fn credential_path(id: &str) -> Result<std::path::PathBuf, String> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err("Invalid account ID".into());
    }
    Ok(crate::app_settings::app_data_directory()
        .join("accounts")
        .join(format!("{id}.json")))
}

fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes
            .len()
            .try_into()
            .map_err(|_| "Credentials are too large")?,
        pbData: bytes.as_ptr() as *mut _,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        let result = if encrypt {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        result.map_err(|_| "Unable to unlock account credentials. Sign in again")?;
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(output.pbData as *mut _));
        Ok(bytes)
    }
}

fn read(id: &str) -> Result<Tokens, String> {
    let file = std::fs::read(credential_path(id)?).map_err(|_| "Sign in again")?;
    let encrypted: Vec<u8> =
        serde_json::from_slice(&file).map_err(|_| "Invalid credential file")?;
    let mut plaintext = crypt(&encrypted, false)?;
    let result = serde_json::from_slice(&plaintext).map_err(|_| "Invalid credential file".into());
    plaintext.fill(0);
    result
}

fn write(id: &str, tokens: &Tokens) -> Result<(), String> {
    let mut plaintext = serde_json::to_vec(tokens).map_err(|_| "Unable to encode credentials")?;
    let encrypted = crypt(&plaintext, true);
    plaintext.fill(0);
    crate::app_settings::write_json_atomic(&credential_path(id)?, &encrypted?)
}

pub fn store(id: &str, tokens: &Tokens) -> Result<(), String> {
    let _guard = VaultLock::acquire(id)?;
    write(id, tokens)
}

pub fn remove(id: &str) -> Result<(), String> {
    let _guard = VaultLock::acquire(id)?;
    match std::fs::remove_file(credential_path(id)?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Unable to remove account credentials".into()),
    }
}

pub fn with_token<T>(
    id: &str,
    fetch: impl Fn(&str, &str) -> Result<T, PollError>,
) -> Result<T, PollError> {
    let _guard = VaultLock::acquire(id).map_err(|_| PollError::RequestFailed)?;
    let mut tokens = read(id).map_err(|_| PollError::NoCredentials)?;
    if tokens.invalid {
        return Err(PollError::AuthRequired);
    }
    let refreshed = tokens.expires_at <= now_unix() + 60;
    if refreshed {
        refresh(id, &mut tokens)?;
    }
    match fetch(&tokens.access_token, &tokens.account_id) {
        Err(PollError::AuthRequired) if !refreshed => {
            refresh(id, &mut tokens)?;
            fetch(&tokens.access_token, &tokens.account_id)
        }
        result => result,
    }
}

fn refresh(id: &str, tokens: &mut Tokens) -> Result<(), PollError> {
    let result = refresh_at(&format!("{ISSUER}/oauth/token"), tokens);
    if result.is_ok() || tokens.invalid {
        write(id, tokens).map_err(|_| PollError::RequestFailed)?;
    }
    result
}

fn refresh_at(endpoint: &str, tokens: &mut Tokens) -> Result<(), PollError> {
    let result = agent().map_err(|_| PollError::RequestFailed)?.post(endpoint)
        .send_json(serde_json::json!({"client_id": CLIENT_ID, "grant_type": "refresh_token", "refresh_token": tokens.refresh_token}));
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => {
            let body: serde_json::Value = response.into_json().unwrap_or_default();
            let code = body["error"]
                .as_str()
                .or_else(|| body["error"]["code"].as_str())
                .unwrap_or("");
            if status == 401
                || matches!(
                    code,
                    "invalid_grant"
                        | "refresh_token_expired"
                        | "refresh_token_reused"
                        | "refresh_token_invalidated"
                )
            {
                tokens.invalid = true;
                return Err(PollError::AuthRequired);
            }
            return Err(PollError::RequestFailed);
        }
        Err(_) => return Err(PollError::RequestFailed),
    };
    let response: TokenResponse = response.into_json().map_err(|_| PollError::RequestFailed)?;
    if response.access_token.is_empty() {
        return Err(PollError::RequestFailed);
    }
    if let Ok(payload) = claims(&response.access_token) {
        if let Some(workspace) =
            payload["https://api.openai.com/auth"]["chatgpt_account_id"].as_str()
        {
            if workspace != tokens.account_id {
                return Err(PollError::AuthRequired);
            }
        }
    }
    tokens.expires_at = expiration(&response.access_token, response.expires_in);
    tokens.access_token = response.access_token;
    if let Some(refresh) = response.refresh_token.filter(|s| !s.is_empty()) {
        tokens.refresh_token = refresh;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(value: serde_json::Value) -> String {
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap())
        )
    }

    #[test]
    fn full_login_uses_callback_and_token_exchange_without_cli() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let token_server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/oauth/token", token_server.server_addr());
        let worker = std::thread::spawn(move || {
            let mut request = token_server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            let form: std::collections::HashMap<_, _> =
                url::form_urlencoded::parse(body.as_bytes())
                    .into_owned()
                    .collect();
            assert_eq!(form["grant_type"], "authorization_code");
            assert_eq!(form["code"], "test-code");
            assert_eq!(form["code_verifier"].len(), 43);
            let response = serde_json::json!({
                "access_token": jwt(serde_json::json!({"exp": now_unix() + 3600})),
                "refresh_token": "test-refresh", "expires_in": 3600,
                "id_token": jwt(serde_json::json!({"sub":"user1","email":"test@example.invalid", "https://api.openai.com/auth":{"chatgpt_account_id":"workspace1"}}))
            });
            request
                .respond(tiny_http::Response::from_string(response.to_string()))
                .unwrap();
        });
        let result = login_at(
            Account::new(0),
            &AtomicBool::new(false),
            server,
            &endpoint,
            Duration::from_secs(5),
            |auth_url| {
                let auth = url::Url::parse(auth_url).unwrap();
                let pairs: std::collections::HashMap<_, _> =
                    auth.query_pairs().into_owned().collect();
                let mut callback = url::Url::parse(&pairs["redirect_uri"]).unwrap();
                callback
                    .query_pairs_mut()
                    .append_pair("state", &pairs["state"])
                    .append_pair("code", "test-code");
                std::thread::spawn(move || {
                    let _ = ureq::get(callback.as_str())
                        .timeout(Duration::from_secs(5))
                        .call();
                });
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(result.account.subject, "user1");
        assert_eq!(result.account.workspace, "workspace1");
        assert_eq!(result.account.identity, "test@example.invalid");
        assert_eq!(result.tokens.refresh_token, "test-refresh");
        worker.join().unwrap();
    }

    #[test]
    fn cancellation_and_timeout_close_callback_listener() {
        for (cancel, timeout, expected) in [
            (true, Duration::from_secs(1), "Sign-in cancelled"),
            (false, Duration::ZERO, "Sign-in timed out. Try again"),
        ] {
            let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
            let result = login_at(
                Account::new(0),
                &AtomicBool::new(cancel),
                server,
                "http://unused",
                timeout,
                |_| Ok(()),
            );
            assert_eq!(result.err().as_deref(), Some(expected));
        }
    }

    #[test]
    fn refresh_rotates_tokens_and_marks_invalid_grant_without_leaking_body() {
        let mut tokens = Tokens {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            account_id: "team".into(),
            expires_at: 0,
            invalid: false,
        };
        for (status, body) in [
            (
                200,
                serde_json::json!({"access_token":"new-access","refresh_token":"new-refresh","expires_in":900}),
            ),
            (
                400,
                serde_json::json!({"error":"invalid_grant", "error_description":"secret-provider-detail"}),
            ),
        ] {
            let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/oauth/token", server.server_addr());
            let expected_refresh = tokens.refresh_token.clone();
            let worker = std::thread::spawn(move || {
                let mut request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                let body_in: serde_json::Value =
                    serde_json::from_reader(request.as_reader()).unwrap();
                assert_eq!(body_in["refresh_token"], expected_refresh);
                request
                    .respond(
                        tiny_http::Response::from_string(body.to_string()).with_status_code(status),
                    )
                    .unwrap();
            });
            let result = refresh_at(&endpoint, &mut tokens);
            if status == 200 {
                assert!(result.is_ok());
                assert_eq!(tokens.access_token, "new-access");
                assert_eq!(tokens.refresh_token, "new-refresh");
                assert!(tokens.expires_at > now_unix());
            } else {
                assert_eq!(result, Err(PollError::AuthRequired));
                assert!(tokens.invalid);
            }
            worker.join().unwrap();
        }
    }

    #[test]
    fn reconnect_cannot_replace_a_different_identity() {
        let mut account = Account::new(0);
        account.subject = "original".into();
        account.workspace = "original-team".into();
        let result = credentials_from_response(
            &mut account,
            TokenResponse {
                access_token: "test".into(),
                refresh_token: Some("test".into()),
                expires_in: Some(3600),
                id_token: Some(jwt(
                    serde_json::json!({"sub":"other", "https://api.openai.com/auth":{"chatgpt_account_id":"other-team"}}),
                )),
            },
        );
        assert!(result.is_err());
        assert_eq!(account.subject, "original");
    }

    #[test]
    fn credential_lock_serializes_refresh_and_remove() {
        let id = Account::new(0).id;
        let guard = VaultLock::acquire(&id).unwrap();
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _guard = VaultLock::acquire(&id).unwrap();
            sender.send(()).unwrap();
        });
        assert!(receiver.recv_timeout(Duration::from_millis(30)).is_err());
        drop(guard);
        receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
    }
    #[test]
    fn pkce_and_callback_are_bound_to_the_login_attempt() {
        let url = authorize_url(
            "http://localhost:1455/auth/callback",
            "expected",
            "verifier",
        );
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(pairs["code_challenge_method"], "S256");
        assert_eq!(
            pairs["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(b"verifier"))
        );
        assert!(callback_code("/auth/callback?state=wrong&code=x", "expected").is_err());
        assert!(callback_code(
            "/auth/callback?state=expected&state=evil&code=x",
            "expected"
        )
        .is_err());
        assert_eq!(
            callback_code("/auth/callback?state=expected&code=a%2Bb", "expected").unwrap(),
            Some("a+b".into())
        );
        assert!(callback_code(
            "/auth/callback?state=expected&error=access_denied",
            "expected"
        )
        .is_err());
        assert_eq!(callback_code("/favicon.ico", "expected").unwrap(), None);
    }
    #[test]
    fn dpapi_roundtrip_and_path_validation() {
        let encrypted = crypt(b"test-credential", true).unwrap();
        assert_ne!(encrypted, b"test-credential");
        assert_eq!(crypt(&encrypted, false).unwrap(), b"test-credential");
        assert!(credential_path("../auth").is_err());
        assert!(credential_path("codex_0123").is_ok());
    }
}
