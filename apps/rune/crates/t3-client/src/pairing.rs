//! Pairing a native client with a T3 server, as the mobile app does
//! (`packages/client-runtime/src/authorization/remote.ts`, `connection/onboarding.ts`):
//! descriptor check → OAuth token exchange for a bearer → a fresh WebSocket ticket per connect.
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::protocol::EnvironmentDescriptor;

pub const ORCHESTRATION_PROTOCOL_VERSION: u32 = 2;

/// What a pairing link resolves to: the server's HTTP origin and the one-time token.
#[derive(Clone, Debug, PartialEq)]
pub struct PairingLink {
    pub http_base: Url,
    pub token: String,
}

/// Accepts `<origin>/pair#token=…`, `?token=…`, the hosted `https://app.t3.codes/…?host=<backend>#token=…`
/// form and the `t3code://pair?pairingUrl=<encoded>` wrapper (`packages/shared/src/remote.ts`).
/// A bare host plus a separately typed code goes through `PairingLink::with_code`.
pub fn parse_pairing_link(input: &str) -> Result<PairingLink> {
    let input = input.trim();
    let url = Url::parse(input).context("That isn't a pairing link")?;
    if url.scheme() == "t3code" {
        let inner = url
            .query_pairs()
            .find(|(k, _)| k == "pairingUrl")
            .map(|(_, v)| v.into_owned());
        return parse_pairing_link(
            &inner.ok_or_else(|| anyhow!("The app link has no pairingUrl"))?,
        );
    }
    if !matches!(url.scheme(), "http" | "https") {
        bail!("Pairing links start with http:// or https://");
    }
    let fragment_token = url.fragment().and_then(|f| {
        url::form_urlencoded::parse(f.as_bytes())
            .find(|(k, _)| k == "token")
            .map(|(_, v)| v.into_owned())
    });
    let query_token = url
        .query_pairs()
        .find(|(k, _)| k == "token")
        .map(|(_, v)| v.into_owned());
    let token = fragment_token
        .or(query_token)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| anyhow!("The link has no pairing token"))?;
    let host = url
        .query_pairs()
        .find(|(k, _)| k == "host")
        .map(|(_, v)| v.into_owned());
    let base = match host {
        Some(h) => Url::parse(&h)
            .or_else(|_| Url::parse(&format!("https://{h}")))
            .context("The link's host isn't a URL")?,
        None => url,
    };
    Ok(PairingLink {
        http_base: origin(&base)?,
        token,
    })
}

impl PairingLink {
    pub fn with_code(host: &str, code: &str) -> Result<PairingLink> {
        let host = host.trim();
        let base = Url::parse(host)
            .or_else(|_| Url::parse(&format!("http://{host}")))
            .context("That host isn't a URL")?;
        Ok(PairingLink {
            http_base: origin(&base)?,
            token: code.trim().to_owned(),
        })
    }
}

fn origin(url: &Url) -> Result<Url> {
    let mut u = url.clone();
    u.set_path("/");
    u.set_query(None);
    u.set_fragment(None);
    Ok(u)
}

/// A saved pairing. The bearer lasts 30 days by default; there is no refresh, so expiry means
/// pairing again.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Credential {
    pub http_base: String,
    pub environment_id: String,
    pub label: String,
    pub access_token: String,
}

impl Credential {
    pub fn base(&self) -> Result<Url> {
        Url::parse(&self.http_base).context("saved server URL")
    }

    /// `/ws` with the required protocol version, a fresh ticket and analytics-only client fields.
    pub fn ws_url(&self, ticket: &str) -> Result<Url> {
        let mut u = self.base()?.join("ws")?;
        let scheme = if u.scheme() == "https" { "wss" } else { "ws" };
        u.set_scheme(scheme).map_err(|_| anyhow!("bad scheme"))?;
        u.query_pairs_mut()
            .append_pair(
                "orchestrationProtocol",
                &ORCHESTRATION_PROTOCOL_VERSION.to_string(),
            )
            .append_pair("wsTicket", ticket)
            .append_pair("frameParts", "1")
            .append_pair("clientCapabilities", "settingsSecretReferences")
            .append_pair("clientSurface", "desktop")
            .append_pair("clientAppVersion", env!("CARGO_PKG_VERSION"))
            .append_pair("clientDeviceType", "desktop")
            .append_pair("clientOs", client_os())
            .append_pair("connectionMethod", "direct");
        Ok(u)
    }
}

fn client_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("t3-rune/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

#[derive(Deserialize)]
struct ErrorBody {
    code: Option<String>,
    reason: Option<String>,
    message: Option<String>,
}

async fn error_text(res: reqwest::Response) -> String {
    let status = res.status();
    match res.json::<ErrorBody>().await {
        Ok(b) => b
            .message
            .or(b.reason)
            .or(b.code)
            .unwrap_or_else(|| status.to_string()),
        Err(_) => status.to_string(),
    }
}

/// Checks the server before spending a one-time code on it.
pub async fn descriptor(base: &Url) -> Result<EnvironmentDescriptor> {
    let res = http()
        .get(base.join(".well-known/t3/environment")?)
        .send()
        .await
        .context("Couldn't reach the server")?;
    if !res.status().is_success() {
        bail!("The server answered {}", error_text(res).await);
    }
    let d: EnvironmentDescriptor = res
        .json()
        .await
        .context("The server's descriptor didn't parse")?;
    let version = d.orchestration_protocol_version.unwrap_or(1);
    if version != ORCHESTRATION_PROTOCOL_VERSION {
        bail!(
            "This server speaks orchestration protocol {version}; update it to pair (needs {ORCHESTRATION_PROTOCOL_VERSION})"
        );
    }
    Ok(d)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// Redeems a pairing token for a bearer credential (`POST /oauth/token`, RFC 8693 token exchange).
pub async fn pair(link: &PairingLink) -> Result<Credential> {
    let d = descriptor(&link.http_base).await?;
    let os = client_os();
    let form = [
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:token-exchange",
        ),
        ("subject_token", link.token.as_str()),
        (
            "subject_token_type",
            "urn:t3:params:oauth:token-type:environment-bootstrap",
        ),
        (
            "requested_token_type",
            "urn:ietf:params:oauth:token-type:access_token",
        ),
        ("client_label", "T3 Code (Rune)"),
        ("client_device_type", "desktop"),
        ("client_os", os),
    ];
    let res = http()
        .post(link.http_base.join("oauth/token")?)
        .form(&form)
        .send()
        .await
        .context("Couldn't reach the server")?;
    if !res.status().is_success() {
        bail!("Pairing failed: {}", error_text(res).await);
    }
    let t: TokenResponse = res
        .json()
        .await
        .context("The token response didn't parse")?;
    Ok(Credential {
        http_base: link.http_base.to_string(),
        environment_id: d.environment_id,
        label: d.label,
        access_token: t.access_token,
    })
}

#[derive(Deserialize)]
struct TicketResponse {
    ticket: String,
}

/// Errors that mean the saved credential is no good and the user has to pair again.
#[derive(Debug)]
pub struct Unauthorized(pub String);

impl std::fmt::Display for Unauthorized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Unauthorized {}

/// A WebSocket ticket for the `rpc` route, valid five minutes; mint one per connect.
pub async fn ws_ticket(cred: &Credential) -> Result<String> {
    let base = cred.base()?;
    let send = |path: &str| {
        http()
            .post(base.join(path).unwrap())
            .bearer_auth(&cred.access_token)
            .send()
    };
    let mut res = send("api/auth/websocket-ticket/rpc")
        .await
        .context("Couldn't reach the server")?;
    // Older servers only have the unscoped route.
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        res = send("api/auth/websocket-ticket")
            .await
            .context("Couldn't reach the server")?;
    }
    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(Unauthorized(error_text(res).await).into());
    }
    if !res.status().is_success() {
        bail!(
            "Couldn't get a connection ticket: {}",
            error_text(res).await
        );
    }
    Ok(res.json::<TicketResponse>().await?.ticket)
}

/// Where saved pairings live: `$XDG_CONFIG_HOME/t3-rune/credential.json` (owner-only).
pub fn credential_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("t3-rune").join("credential.json"))
}

pub fn load_credential() -> Option<Credential> {
    serde_json::from_slice(&std::fs::read(credential_path()?).ok()?).ok()
}

pub fn save_credential(cred: &Credential) -> Result<()> {
    let path = credential_path().ok_or_else(|| anyhow!("no config directory"))?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(cred)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub fn forget_credential() {
    if let Some(p) = credential_path() {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_link_form() {
        let l = parse_pairing_link("http://localhost:5733/pair#token=abc").unwrap();
        assert_eq!(
            (l.http_base.as_str(), l.token.as_str()),
            ("http://localhost:5733/", "abc")
        );
        assert_eq!(
            parse_pairing_link("https://box.tail.ts.net/pair?token=q")
                .unwrap()
                .token,
            "q"
        );
        let hosted = parse_pairing_link(
            "https://app.t3.codes/pair?host=https%3A%2F%2Fbox.ts.net%3A4443&label=x#token=t",
        )
        .unwrap();
        assert_eq!(hosted.http_base.as_str(), "https://box.ts.net:4443/");
        let wrapped = parse_pairing_link(
            "t3code://pair?pairingUrl=http%3A%2F%2F10.0.0.2%3A3773%2Fpair%23token%3Dz",
        )
        .unwrap();
        assert_eq!(
            (wrapped.http_base.as_str(), wrapped.token.as_str()),
            ("http://10.0.0.2:3773/", "z")
        );
        assert!(parse_pairing_link("http://localhost:5733/").is_err());
    }

    #[test]
    fn ws_url_carries_protocol_and_ticket() {
        let c = Credential {
            http_base: "https://box.ts.net/".into(),
            environment_id: "e".into(),
            label: "l".into(),
            access_token: "a".into(),
        };
        let u = c.ws_url("tk").unwrap();
        assert_eq!((u.scheme(), u.path()), ("wss", "/ws"));
        let q: Vec<_> = u
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert!(q.contains(&("orchestrationProtocol".into(), "2".into())));
        assert!(q.contains(&("wsTicket".into(), "tk".into())));
    }
}
