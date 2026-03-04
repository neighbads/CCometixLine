use super::{Segment, SegmentData};
use crate::config::{InputData, SegmentId};
use crate::utils::logger::log_debug;
use std::collections::HashMap;
use std::process::Command;

pub struct CustomSegment {
    name: String,
    command: String,
    #[allow(dead_code)]
    timeout_secs: u64,
}

impl CustomSegment {
    pub fn new(name: String, command: String, timeout_secs: u64) -> Self {
        Self {
            name,
            command,
            timeout_secs,
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

        log_debug(
            "custom",
            &format!(
                "segment '{}' executing command: {}",
                self.name, self.command
            ),
        );

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

        let output = match cmd.output() {
            Ok(output) => output,
            Err(e) => {
                log_debug(
                    "custom",
                    &format!("segment '{}' command failed: {}", self.name, e),
                );
                return None;
            }
        };

        if !output.status.success() {
            log_debug(
                "custom",
                &format!(
                    "segment '{}' exited with status: {}",
                    self.name, output.status
                ),
            );
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let primary = stdout.lines().next().unwrap_or("").trim().to_string();

        if primary.is_empty() {
            log_debug(
                "custom",
                &format!("segment '{}' returned empty output, hiding", self.name),
            );
            return None;
        }

        log_debug(
            "custom",
            &format!("segment '{}' result: {}", self.name, primary),
        );

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
