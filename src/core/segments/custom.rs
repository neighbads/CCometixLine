use super::{Segment, SegmentData};
use crate::config::{InputData, SegmentId};
use crate::utils::logger::log_debug;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc;

#[derive(Debug, Serialize, Deserialize)]
struct CustomCache {
    output: String,
    cached_at: String,
}

pub struct CustomSegment {
    name: String,
    command: String,
    timeout_secs: u64,
    cache_duration: u64,
}

impl CustomSegment {
    pub fn new(name: String, command: String, timeout_secs: u64, cache_duration: u64) -> Self {
        Self {
            name,
            command,
            timeout_secs,
            cache_duration,
        }
    }

    fn get_cache_path(&self) -> Option<std::path::PathBuf> {
        let home = dirs::home_dir()?;
        Some(
            home.join(".claude")
                .join("ccline")
                .join(format!(".custom_cache_{}.json", self.name)),
        )
    }

    fn load_cache(&self) -> Option<CustomCache> {
        let cache_path = self.get_cache_path()?;
        if !cache_path.exists() {
            log_debug("custom:cache", &format!("'{}': no cache file", self.name));
            return None;
        }
        let content = std::fs::read_to_string(&cache_path).ok()?;
        match serde_json::from_str(&content) {
            Ok(cache) => {
                log_debug("custom:cache", &format!("'{}': loaded cache", self.name));
                Some(cache)
            }
            Err(e) => {
                log_debug(
                    "custom:cache",
                    &format!("'{}': parse error: {}", self.name, e),
                );
                None
            }
        }
    }

    fn save_cache(&self, output: &str) {
        if let Some(cache_path) = self.get_cache_path() {
            if let Some(parent) = cache_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let cache = CustomCache {
                output: output.to_string(),
                cached_at: Utc::now().to_rfc3339(),
            };
            if let Ok(json) = serde_json::to_string_pretty(&cache) {
                let _ = std::fs::write(&cache_path, json);
            }
        }
    }

    fn is_cache_valid(&self, cache: &CustomCache) -> bool {
        if let Ok(cached_at) = DateTime::parse_from_rfc3339(&cache.cached_at) {
            let elapsed = Utc::now()
                .signed_duration_since(cached_at.with_timezone(&Utc))
                .num_seconds();
            let valid = elapsed < self.cache_duration as i64;
            log_debug(
                "custom:cache",
                &format!(
                    "'{}': age={}s, max={}s, valid={}",
                    self.name, elapsed, self.cache_duration, valid
                ),
            );
            valid
        } else {
            false
        }
    }

    fn execute_command(&self, input: &InputData) -> Option<String> {
        let env_vars = Self::build_env_vars(input);

        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            c.args(["/C", &self.command]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", &self.command]);
            c
        };

        for (key, value) in &env_vars {
            cmd.env(key, value);
        }

        // Spawn child process
        let child = match cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                log_debug(
                    "custom",
                    &format!("'{}' spawn failed: {}", self.name, e),
                );
                return None;
            }
        };

        // Wait with timeout using a channel
        let timeout = std::time::Duration::from_secs(self.timeout_secs);
        let (tx, rx) = mpsc::channel();
        let child_id = child.id();

        std::thread::spawn(move || {
            let result = child.wait_with_output();
            let _ = tx.send(result);
        });

        match rx.recv_timeout(timeout) {
            Ok(Ok(output)) => {
                if !output.status.success() {
                    log_debug(
                        "custom",
                        &format!("'{}' exited with status: {}", self.name, output.status),
                    );
                    return None;
                }
                let stdout = String::from_utf8_lossy(&output.stdout);
                let primary = stdout.lines().next().unwrap_or("").trim().to_string();

                if primary.is_empty() {
                    log_debug(
                        "custom",
                        &format!("'{}' returned empty output, hiding", self.name),
                    );
                    return None;
                }

                log_debug("custom", &format!("'{}' result: {}", self.name, primary));
                Some(primary)
            }
            Ok(Err(e)) => {
                log_debug(
                    "custom",
                    &format!("'{}' wait error: {}", self.name, e),
                );
                None
            }
            Err(_) => {
                // Timed out — kill the process
                #[cfg(unix)]
                {
                    use std::process::Command as KillCmd;
                    let _ = KillCmd::new("kill").arg("-9").arg(child_id.to_string()).output();
                }
                log_debug(
                    "custom",
                    &format!(
                        "'{}' timed out after {}s, killed",
                        self.name, self.timeout_secs
                    ),
                );
                None
            }
        }
    }

    fn build_env_vars(input: &InputData) -> Vec<(String, String)> {
        let mut vars = vec![
            ("CCLINE_MODEL_ID".to_string(), input.model.id.clone()),
            (
                "CCLINE_MODEL_NAME".to_string(),
                input.model.display_name.clone(),
            ),
            (
                "CCLINE_CURRENT_DIR".to_string(),
                input.workspace.current_dir.clone(),
            ),
        ];

        if let Some(cost) = &input.cost {
            if let Some(cost_usd) = cost.total_cost_usd {
                vars.push(("CCLINE_COST_USD".to_string(), format!("{:.4}", cost_usd)));
            }
            if let Some(duration_ms) = cost.total_duration_ms {
                vars.push(("CCLINE_DURATION_MS".to_string(), duration_ms.to_string()));
            }
        }

        vars
    }
}

impl Segment for CustomSegment {
    fn collect(&self, input: &InputData) -> Option<SegmentData> {
        if self.command.is_empty() {
            log_debug(
                "custom",
                &format!("segment '{}' skipped: empty command", self.name),
            );
            return None;
        }

        // Check cache first
        let cached = self.load_cache();
        let primary = if cached.as_ref().is_some_and(|c| self.is_cache_valid(c)) {
            let output = cached.unwrap().output;
            log_debug("custom", &format!("'{}' using cached: {}", self.name, output));
            output
        } else {
            log_debug(
                "custom",
                &format!("'{}' executing: {}", self.name, self.command),
            );
            match self.execute_command(input) {
                Some(output) => {
                    self.save_cache(&output);
                    output
                }
                None => {
                    // Fall back to stale cache if command fails
                    if let Some(cache) = cached {
                        log_debug(
                            "custom",
                            &format!("'{}' command failed, using stale cache", self.name),
                        );
                        cache.output
                    } else {
                        return None;
                    }
                }
            }
        };

        let mut metadata = HashMap::new();
        metadata.insert("command".to_string(), self.command.clone());
        metadata.insert("custom_name".to_string(), self.name.clone());

        Some(SegmentData {
            primary,
            secondary: String::new(),
            metadata,
        })
    }

    fn id(&self) -> SegmentId {
        SegmentId::Custom(self.name.clone())
    }
}
