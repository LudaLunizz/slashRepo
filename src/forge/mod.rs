//! Fetching one repo from GitHub (through `gh`), GitLab (any host) or Codeberg.
//! All network access goes through [`Net`], so the tests can answer with fixed data.
//!
//! Each forge's answer is read into typed structs (private to its module) and turned into a
//! [`Fetched`] in one place. An answer of another shape is a [`ForgeError::BadAnswer`], never a
//! card with empty fields, and a CI state this program does not know stays visible as
//! [`CheckState::Unknown`].

use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::config::CODEBERG;
use crate::report::Fetched;

mod codeberg;
mod github;
mod gitlab;

/// Why a repo could not be read. The text says what went wrong; [`ForgeError::fix`] says what to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForgeError {
    /// 401 or 403: no token, or one without access to this repo
    Auth(String),
    /// the forge says there is no such repo (or it is private and we may not see it)
    NotFound(String),
    /// no answer: network down, timeout, `gh` missing or hung
    Unreachable(String),
    /// an answer of a shape this program does not read
    BadAnswer(String),
    /// the forge (or `gh`) said no for another reason; its own words
    Refused(String),
    /// the check itself crashed (a bug here, not at the forge)
    Internal,
}

pub type Res<T> = Result<T, ForgeError>;

impl fmt::Display for ForgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForgeError::Auth(m) => write!(f, "access refused: {m}"),
            ForgeError::NotFound(m) | ForgeError::Refused(m) => f.write_str(m),
            ForgeError::Unreachable(m) => write!(f, "no answer: {m}"),
            ForgeError::BadAnswer(m) => write!(f, "unexpected answer: {m}"),
            ForgeError::Internal => f.write_str("internal error while reading the answer"),
        }
    }
}

impl ForgeError {
    /// What the user can do about it, shown as the card's to-do.
    pub fn fix(&self, forge: Forge) -> String {
        let fix = match (self, forge) {
            (ForgeError::Auth(_), Forge::GitHub) => "run `gh auth login`, or check `gh auth status`",
            (ForgeError::Auth(_), Forge::GitLab) => "set GITLAB_TOKEN (and GITLAB_HOST if it is not gitlab.com)",
            (ForgeError::Auth(_), Forge::Codeberg) => "set CODEBERG_TOKEN to a token that can read this repo",
            (ForgeError::NotFound(_), Forge::GitHub) => {
                "check the name; a private repo needs a `gh` login that can see it"
            }
            (ForgeError::Unreachable(_), Forge::GitHub) => "check the network and that `gh` is installed, then Refresh",
            (ForgeError::Unreachable(_), _) => "check the network, then Refresh",
            (ForgeError::BadAnswer(_), _) => "the forge changed its answer: please report this to slashRepo",
            (ForgeError::Internal, _) => "press Refresh to try again",
            (_, Forge::GitHub) => "gh call failed: check `gh auth status`",
            (_, Forge::GitLab) => "GitLab call failed: check the name, or GITLAB_TOKEN for a private project",
            (_, Forge::Codeberg) => "Codeberg call failed: check the name, or CODEBERG_TOKEN for a private repo",
        };
        fix.into()
    }
}

/// Which forge a repos-file line points at: GitHub lines are owner/name, the others carry a host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    GitLab,
    Codeberg,
}

/// GitHub owners have no dots, hosts do.
pub fn host(full: &str) -> Option<&str> {
    let first = full.split('/').next().unwrap_or("");
    first.contains('.').then_some(first)
}

impl Forge {
    pub fn of(full: &str) -> Forge {
        match host(full) {
            None => Forge::GitHub,
            Some(CODEBERG) => Forge::Codeberg,
            Some(_) => Forge::GitLab,
        }
    }

    pub fn fetch(self, net: &dyn Net, full: &str) -> Res<Fetched> {
        match self {
            Forge::GitHub => github::fetch(net, full),
            Forge::GitLab => gitlab::fetch(net, full),
            Forge::Codeberg => codeberg::fetch(net, full),
        }
    }
}

/// A response body as JSON plus the one header we read (`X-Total-Count`).
pub struct Reply {
    pub json: Value,
    pub total: Option<u64>,
}

pub trait Net: Sync {
    /// `gh ARGS...`, stdout parsed as JSON.
    fn gh(&self, args: &[&str]) -> Res<Value>;
    /// GET (body None) or POST json to url, with an optional Authorization header.
    fn http(&self, url: &str, auth: Option<&str>, body: Option<&Value>) -> Res<Reply>;
    /// An environment variable (tokens, GITLAB_HOST); empty counts as unset.
    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }
}

pub struct RealNet {
    agent: ureq::Agent,
}

impl RealNet {
    pub fn new() -> Self {
        let config =
            ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).user_agent("slashRepo");
        // Windows: schannel and the Windows certificate store (no bundled roots in the exe)
        #[cfg(windows)]
        let config = config.tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        );
        let agent = config.build().new_agent();
        RealNet { agent }
    }
}

/// The same 60 seconds the HTTPS calls get.
const GH_TIMEOUT: Duration = Duration::from_secs(60);

/// `gh`'s own words on stderr, sorted into the error kinds.
fn gh_error(stderr: &str) -> ForgeError {
    let msg = if stderr.is_empty() { "gh failed".to_string() } else { stderr.to_string() };
    let low = stderr.to_lowercase();
    if low.contains("http 401") || low.contains("http 403") || low.contains("gh auth login") {
        ForgeError::Auth(msg)
    } else if low.contains("could not resolve to a repository") || low.contains("http 404") {
        ForgeError::NotFound(msg)
    } else {
        ForgeError::Refused(msg)
    }
}

fn ureq_error(url: &str, e: ureq::Error) -> ForgeError {
    match e {
        ureq::Error::StatusCode(c @ (401 | 403)) => ForgeError::Auth(format!("HTTP {c} from {url}")),
        ureq::Error::StatusCode(404) => ForgeError::NotFound(format!("not found: {url}")),
        ureq::Error::StatusCode(c) => ForgeError::Refused(format!("HTTP {c} from {url}")),
        e => ForgeError::Unreachable(e.to_string()),
    }
}

impl Net for RealNet {
    fn gh(&self, args: &[&str]) -> Res<Value> {
        let mut cmd = crate::command("gh");
        cmd.args(args);
        let out = crate::output_within(cmd, GH_TIMEOUT).map_err(|e| ForgeError::Unreachable(format!("gh: {e}")))?;
        if !out.status.success() {
            return Err(gh_error(String::from_utf8_lossy(&out.stderr).trim()));
        }
        serde_json::from_slice(&out.stdout).map_err(|e| ForgeError::BadAnswer(format!("gh output: {e}")))
    }

    fn http(&self, url: &str, auth: Option<&str>, body: Option<&Value>) -> Res<Reply> {
        let resp = match body {
            Some(b) => {
                let mut req = self.agent.post(url).header("Content-Type", "application/json");
                if let Some(a) = auth {
                    req = req.header("Authorization", a);
                }
                req.send(b.to_string())
            }
            None => {
                let mut req = self.agent.get(url);
                if let Some(a) = auth {
                    req = req.header("Authorization", a);
                }
                req.call()
            }
        };
        let mut resp = resp.map_err(|e| ureq_error(url, e))?;
        let total = resp.headers().get("x-total-count").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
        let text = resp.body_mut().read_to_string().map_err(|e| ForgeError::Unreachable(e.to_string()))?;
        let json =
            serde_json::from_str(&text).map_err(|e| ForgeError::BadAnswer(format!("bad JSON from {url}: {e}")))?;
        Ok(Reply { json, total })
    }
}

/// Percent-encode everything except unreserved characters and the ones in `safe`.
pub fn quote(s: &str, safe: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) || safe.as_bytes().contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Read a reply into its typed shape; any other shape is a BadAnswer naming what was read.
fn parse<T: DeserializeOwned>(what: &str, v: Value) -> Res<T> {
    serde_json::from_value(v).map_err(|e| ForgeError::BadAnswer(format!("{what}: {e}")))
}

// ---- shapes shared by the forges' answers ----

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
struct Name {
    name: String,
}

#[derive(Deserialize)]
struct Title {
    title: String,
}

#[derive(Deserialize)]
struct Login {
    login: String,
}

#[derive(Deserialize)]
struct Username {
    username: String,
}

/// Issue and MR numbers: GitHub/Codeberg send numbers, GitLab's GraphQL sends strings.
#[derive(Deserialize)]
#[serde(untagged)]
enum Id {
    N(u64),
    S(String),
}

impl Id {
    fn text(self) -> String {
        match self {
            Id::N(n) => n.to_string(),
            Id::S(s) => s,
        }
    }
}

/// A [`Net`] that answers from a function, shared by the forge modules' tests.
#[cfg(test)]
mod fake {
    use super::{Net, Reply, Res};
    use serde_json::Value;
    use std::sync::Mutex;

    /// Answers by URL (or the gh arguments) and records (url, Authorization) of every HTTP call.
    pub struct Fake {
        pub sent: Mutex<Vec<(String, Option<String>)>>,
        pub answer: fn(&str) -> Reply,
        pub env: Vec<(&'static str, &'static str)>,
    }

    impl Net for Fake {
        fn gh(&self, args: &[&str]) -> Res<Value> {
            Ok((self.answer)(&args.join(" ")).json)
        }
        fn http(&self, url: &str, auth: Option<&str>, _: Option<&Value>) -> Res<Reply> {
            self.sent.lock().unwrap().push((url.into(), auth.map(String::from)));
            Ok((self.answer)(url))
        }
        fn env(&self, key: &str) -> Option<String> {
            self.env.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string())
        }
    }

    pub fn fake(answer: fn(&str) -> Reply) -> Fake {
        Fake { sent: Mutex::new(vec![]), answer, env: vec![] }
    }

    pub fn reply(json: Value) -> Reply {
        Reply { json, total: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh_errors_name_the_fix() {
        assert!(matches!(gh_error("HTTP 401: Bad credentials"), ForgeError::Auth(_)));
        assert!(matches!(gh_error("To get started with GitHub CLI, please run:  gh auth login"), ForgeError::Auth(_)));
        assert!(matches!(
            gh_error("GraphQL: Could not resolve to a Repository with the name 'o/x'. (repository)"),
            ForgeError::NotFound(_)
        ));
        assert_eq!(gh_error(""), ForgeError::Refused("gh failed".into()));
        assert_eq!(ForgeError::Auth("x".into()).fix(Forge::GitHub), "run `gh auth login`, or check `gh auth status`");
        assert_eq!(ForgeError::Auth("x".into()).to_string(), "access refused: x");
    }

    #[test]
    fn forge_of_a_line() {
        assert_eq!(Forge::of("me/app"), Forge::GitHub);
        assert_eq!(Forge::of("codeberg.org/me/app"), Forge::Codeberg);
        assert_eq!(Forge::of("git.example.org/g/p"), Forge::GitLab);
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("g/sub/p", ""), "g%2Fsub%2Fp");
        assert_eq!(quote("feature/x y", "/"), "feature/x%20y");
    }
}
