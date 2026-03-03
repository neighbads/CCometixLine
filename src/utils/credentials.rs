use super::logger::log_debug;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize)]
struct OAuthCredentials {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<u64>,
    scopes: Option<Vec<String>>,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<OAuthCredentials>,
}

pub fn get_oauth_token() -> Option<String> {
    if cfg!(target_os = "macos") {
        get_oauth_token_macos()
    } else {
        get_oauth_token_file()
    }
}

fn get_oauth_token_macos() -> Option<String> {
    use std::process::Command;

    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());

    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-a",
            &user,
            "-w",
            "-s",
            "Claude Code-credentials",
        ])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let json_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json_str.is_empty() {
                if let Ok(creds_file) = serde_json::from_str::<CredentialsFile>(&json_str) {
                    let token = creds_file.claude_ai_oauth.map(|oauth| oauth.access_token);
                    if token.is_some() {
                        log_debug("credentials", "token found via macOS keychain");
                    } else {
                        log_debug("credentials", "keychain entry has no claudeAiOauth");
                    }
                    return token;
                }
            }
            log_debug("credentials", "keychain entry empty or unparseable");
            None
        }
        _ => {
            log_debug(
                "credentials",
                "macOS keychain lookup failed, falling back to file",
            );
            // Fallback to file-based credentials
            get_oauth_token_file()
        }
    }
}

fn get_oauth_token_file() -> Option<String> {
    // Try CLAUDE_CONFIG_DIR first if set (respects explicit user configuration)
    if let Ok(config_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        let config_path = PathBuf::from(&config_dir).join(".credentials.json");
        log_debug(
            "credentials",
            &format!("trying CLAUDE_CONFIG_DIR: {}", config_dir),
        );
        if let Some(token) = read_token_from_path(&config_path) {
            log_debug("credentials", "token found via CLAUDE_CONFIG_DIR");
            return Some(token);
        }
    }

    // Fall back to default ~/.claude/.credentials.json
    if let Some(default_path) = get_credentials_path() {
        log_debug(
            "credentials",
            &format!("trying default path: {}", default_path.display()),
        );
        if let Some(token) = read_token_from_path(&default_path) {
            log_debug("credentials", "token found via default credentials path");
            return Some(token);
        }
    }

    log_debug("credentials", "no token found from any source");
    None
}

fn get_credentials_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".claude").join(".credentials.json"))
}

/// Read OAuth token from a credentials file path
fn read_token_from_path(path: &PathBuf) -> Option<String> {
    if !path.exists() {
        log_debug(
            "credentials",
            &format!("file not found: {}", path.display()),
        );
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    let creds_file: CredentialsFile = serde_json::from_str(&content).ok()?;

    creds_file.claude_ai_oauth.map(|oauth| oauth.access_token)
}
