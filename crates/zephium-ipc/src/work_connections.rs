//! Connections Settings lists: command-line tools the agent can use and the
//! MCP servers the person added. Secrets never cross: the response says only
//! which ones the Keychain holds.
use super::*;

pub const MAX_WORK_SERVERS: usize = 32;
pub const MAX_WORK_SERVER_ARGS: usize = 64;
pub const MAX_WORK_SERVER_ENV: usize = 32;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkCliStatusV1 {
    /// The command broker does not admit this connection or its status probe.
    Unavailable,
    SignedIn,
    SignedOut,
    /// Installed and needs no account (git).
    Ready,
    Missing,
    Unknown,
}

/// A command-line tool as Settings shows it.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(deny_unknown_fields)]
pub struct WorkCliRowV1 {
    /// `gh`, `git`, `codex` or `claude`.
    pub id: String,
    pub status: WorkCliStatusV1,
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The account the tool says it uses: a login, an email, "ChatGPT".
    pub account: Option<String>,
}

/// One environment variable a stdio server gets.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(deny_unknown_fields)]
pub struct WorkServerEnvV1 {
    pub name: String,
    /// Kept in the Keychain; `value` is then always absent.
    pub secret: bool,
    pub value: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkServerAuthV1 {
    None,
    /// A token the person pasted, sent as `Authorization: Bearer`.
    Bearer,
    /// Sign in with the service in the browser.
    #[serde(rename = "oauth")]
    OAuth,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkServerTransportV1 {
    Stdio {
        /// A program name found on the login shell's PATH, or an absolute path.
        command: String,
        args: Vec<String>,
        env: Vec<WorkServerEnvV1>,
    },
    Http {
        url: String,
        auth: WorkServerAuthV1,
    },
}

/// An MCP server the person added, as stored per profile.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(deny_unknown_fields)]
pub struct WorkServerV1 {
    /// Lowercase letters, digits and dashes; namespaces its tools.
    pub id: String,
    /// "Linear", "Notion".
    pub name: String,
    pub transport: WorkServerTransportV1,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(deny_unknown_fields)]
pub struct WorkServerRowV1 {
    pub server: WorkServerV1,
    /// Secret names the Keychain holds: `bearer`, `env.NAME`.
    pub secrets: Vec<String>,
    /// OAuth credentials are held.
    pub signed_in: bool,
}

/// A new secret for a server; an absent one keeps what the Keychain holds.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(deny_unknown_fields)]
pub struct WorkServerSecretV1 {
    /// `bearer` or `env.NAME`.
    pub account: String,
    pub value: String,
}

/// Adds a server, or replaces the one with the same id.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(deny_unknown_fields)]
pub struct WorkServerDraftV1 {
    pub server: WorkServerV1,
    pub secrets: Vec<WorkServerSecretV1>,
    /// The id it had before an edit renamed it.
    pub previous: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(deny_unknown_fields)]
pub struct WorkConnectionsResponseV1 {
    pub version: u16,
    pub profile: String,
    pub clis: Vec<WorkCliRowV1>,
    pub servers: Vec<WorkServerRowV1>,
    pub error: Option<WorkFailureV1>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkServerOutcomeV1 {
    Ready,
    /// The server wants the person to sign in.
    SignIn,
    /// Sign-in was closed or timed out.
    Cancelled,
    /// The program was not found or could not start.
    NotFound,
    /// It did not answer in time.
    Timeout,
    /// It answered outside the protocol or closed.
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(deny_unknown_fields)]
pub struct WorkServerToolV1 {
    pub name: String,
    pub title: Option<String>,
    /// Calls stop for a Confirm.
    pub asks: bool,
}

/// What testing a server found.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Type)]
#[serde(deny_unknown_fields)]
pub struct WorkServerCheckV1 {
    pub version: u16,
    pub profile: String,
    pub id: String,
    pub outcome: WorkServerOutcomeV1,
    /// The name the server gives itself.
    pub server_name: Option<String>,
    pub tools: Vec<WorkServerToolV1>,
    pub error: Option<WorkFailureV1>,
}

fn words(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control)
}

impl WorkServerV1 {
    pub fn validate(&self) -> bool {
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 40
            && !self.id.starts_with('-')
            && !self.id.ends_with('-')
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        id_ok
            && words(&self.name, 40)
            && match &self.transport {
                WorkServerTransportV1::Stdio { command, args, env } => {
                    words(command, 512)
                        && args.len() <= MAX_WORK_SERVER_ARGS
                        && args
                            .iter()
                            .all(|a| a.len() <= 1024 && !a.chars().any(|c| c.is_control()))
                        && env.len() <= MAX_WORK_SERVER_ENV
                        && env.iter().all(|e| {
                            let name_ok = !e.name.is_empty()
                                && e.name.len() <= 64
                                && !e.name.starts_with(|c: char| c.is_ascii_digit())
                                && e.name.bytes().all(|b| {
                                    b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'
                                });
                            name_ok
                                && (!e.secret || e.value.is_none())
                                && e.value
                                    .as_ref()
                                    .is_none_or(|v| v.len() <= 4096 && !v.contains('\0'))
                        })
                        && {
                            let mut names: Vec<&str> =
                                env.iter().map(|e| e.name.as_str()).collect();
                            names.sort_unstable();
                            names.windows(2).all(|w| w[0] != w[1])
                        }
                }
                WorkServerTransportV1::Http { url, .. } => {
                    words(url, 2048) && (url.starts_with("https://") || url.starts_with("http://"))
                }
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(id: &str, env: Vec<WorkServerEnvV1>) -> WorkServerV1 {
        WorkServerV1 {
            id: id.into(),
            name: "Notion".into(),
            transport: WorkServerTransportV1::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "@notionhq/notion-mcp-server".into()],
                env,
            },
            enabled: true,
        }
    }

    #[test]
    fn servers_validate() {
        let token = WorkServerEnvV1 {
            name: "NOTION_TOKEN".into(),
            secret: true,
            value: None,
        };
        assert!(stdio("notion", vec![token.clone()]).validate());
        assert!(!stdio("Notion", vec![]).validate());
        assert!(!stdio("-x", vec![]).validate());
        let leaked = WorkServerEnvV1 {
            value: Some("secret".into()),
            ..token.clone()
        };
        assert!(!stdio("notion", vec![leaked]).validate());
        assert!(!stdio("notion", vec![token.clone(), token]).validate());
        let lower = WorkServerEnvV1 {
            name: "path".into(),
            secret: false,
            value: Some("x".into()),
        };
        assert!(!stdio("notion", vec![lower]).validate());
        let json = serde_json::to_value(stdio("notion", vec![])).unwrap();
        assert_eq!(json["transport"]["kind"], "stdio");
        let http = WorkServerV1 {
            id: "linear".into(),
            name: "Linear".into(),
            transport: WorkServerTransportV1::Http {
                url: "https://mcp.linear.app/mcp".into(),
                auth: WorkServerAuthV1::OAuth,
            },
            enabled: true,
        };
        assert!(http.validate());
        assert_eq!(
            serde_json::to_value(&http).unwrap()["transport"]["auth"],
            "oauth"
        );
    }
}
