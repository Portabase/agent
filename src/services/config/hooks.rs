use serde::{Deserialize, Serialize};

pub const HOOK_DEFAULT_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HookCommand {
    pub command: Vec<String>,
    #[serde(default)]
    pub container: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub pre: Option<HookCommand>,
    #[serde(default)]
    pub post: Option<HookCommand>,
    #[serde(default)]
    pub on_success: Option<HookCommand>,
    #[serde(default)]
    pub on_failure: Option<HookCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Pre,
    Post,
    OnSuccess,
    OnFailure,
}

impl Phase {
    pub const ALL: [Phase; 4] = [Phase::Pre, Phase::Post, Phase::OnSuccess, Phase::OnFailure];

    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Pre => "pre",
            Phase::Post => "post",
            Phase::OnSuccess => "on_success",
            Phase::OnFailure => "on_failure",
        }
    }

    pub fn command(self, hooks: &Hooks) -> Option<&HookCommand> {
        match self {
            Phase::Pre => hooks.pre.as_ref(),
            Phase::Post => hooks.post.as_ref(),
            Phase::OnSuccess => hooks.on_success.as_ref(),
            Phase::OnFailure => hooks.on_failure.as_ref(),
        }
    }
}

impl Hooks {
    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs.unwrap_or(HOOK_DEFAULT_TIMEOUT_SECS)
    }

    pub(super) fn validate(&self, db_name: &str) -> Result<(), String> {
        for p in Phase::ALL {
            let phase = p.as_str();
            if let Some(hook) = p.command(self) {
                if hook
                    .command
                    .first()
                    .is_none_or(|program| program.trim().is_empty())
                {
                    return Err(format!("Empty command in hooks.{phase} for '{db_name}'"));
                }
                if hook
                    .container
                    .as_deref()
                    .is_some_and(|c| c.trim().is_empty())
                {
                    return Err(format!("Empty container in hooks.{phase} for '{db_name}'"));
                }
                if hook.user.is_some() && hook.container.is_none() {
                    return Err(format!(
                        "hooks.{phase} for '{db_name}' sets user without container (user only applies to docker exec)"
                    ));
                }
            }
        }
        Ok(())
    }
}
