use super::{Segment, SegmentData};
use crate::config::{InputData, SegmentId};
use crate::utils::credentials;
use crate::utils::logger::log_debug;
use chrono::{DateTime, Datelike, Duration, Local, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct ApiUsageResponse {
    five_hour: UsagePeriod,
    seven_day: Option<UsagePeriod>,
}

#[derive(Debug, Deserialize)]
struct UsagePeriod {
    utilization: f64,
    resets_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiUsageCache {
    five_hour_utilization: f64,
    seven_day_utilization: Option<f64>,
    resets_at: Option<String>,
    cached_at: String,
}

#[derive(Default)]
pub struct UsageSegment;

impl UsageSegment {
    pub fn new() -> Self {
        Self
    }

    fn get_circle_icon(utilization: f64) -> String {
        let percent = (utilization * 100.0) as u8;
        match percent {
            0..=12 => "\u{f0a9e}".to_string(),  // circle_slice_1
            13..=25 => "\u{f0a9f}".to_string(), // circle_slice_2
            26..=37 => "\u{f0aa0}".to_string(), // circle_slice_3
            38..=50 => "\u{f0aa1}".to_string(), // circle_slice_4
            51..=62 => "\u{f0aa2}".to_string(), // circle_slice_5
            63..=75 => "\u{f0aa3}".to_string(), // circle_slice_6
            76..=87 => "\u{f0aa4}".to_string(), // circle_slice_7
            _ => "\u{f0aa5}".to_string(),       // circle_slice_8
        }
    }

    fn format_reset_time(reset_time_str: Option<&str>) -> String {
        if let Some(time_str) = reset_time_str {
            if let Ok(dt) = DateTime::parse_from_rfc3339(time_str) {
                let mut local_dt = dt.with_timezone(&Local);
                if local_dt.minute() > 45 {
                    local_dt += Duration::hours(1);
                }
                return format!(
                    "{}-{}-{}",
                    local_dt.month(),
                    local_dt.day(),
                    local_dt.hour()
                );
            }
        }
        "?".to_string()
    }

    fn get_cache_path() -> Option<std::path::PathBuf> {
        let home = dirs::home_dir()?;
        Some(
            home.join(".claude")
                .join("ccline")
                .join(".api_usage_cache.json"),
        )
    }

    fn load_cache(&self) -> Option<ApiUsageCache> {
        let cache_path = Self::get_cache_path()?;
        if !cache_path.exists() {
            log_debug("usage:cache", "no cache file found");
            return None;
        }

        let content = std::fs::read_to_string(&cache_path).ok()?;
        match serde_json::from_str(&content) {
            Ok(cache) => {
                log_debug("usage:cache", "loaded cache successfully");
                Some(cache)
            }
            Err(e) => {
                log_debug("usage:cache", &format!("cache parse error: {}", e));
                None
            }
        }
    }

    fn save_cache(&self, cache: &ApiUsageCache) {
        if let Some(cache_path) = Self::get_cache_path() {
            if let Some(parent) = cache_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(cache) {
                let _ = std::fs::write(&cache_path, json);
            }
        }
    }

    fn is_cache_valid(&self, cache: &ApiUsageCache, cache_duration: u64) -> bool {
        if let Ok(cached_at) = DateTime::parse_from_rfc3339(&cache.cached_at) {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(cached_at.with_timezone(&Utc));
            let valid = elapsed.num_seconds() < cache_duration as i64;
            log_debug(
                "usage:cache",
                &format!(
                    "cache age={}s, max={}s, valid={}",
                    elapsed.num_seconds(),
                    cache_duration,
                    valid
                ),
            );
            valid
        } else {
            log_debug("usage:cache", "could not parse cached_at timestamp");
            false
        }
    }

    fn get_claude_code_version() -> String {
        use std::process::Command;

        let output = Command::new("npm")
            .args(["view", "@anthropic-ai/claude-code", "version"])
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !version.is_empty() {
                    return format!("claude-code/{}", version);
                }
            }
            _ => {}
        }

        "claude-code".to_string()
    }

    fn get_proxy_from_settings() -> Option<String> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()?;
        let settings_path = format!("{}/.claude/settings.json", home);

        let content = std::fs::read_to_string(&settings_path).ok()?;
        let settings: serde_json::Value = serde_json::from_str(&content).ok()?;

        // Try HTTPS_PROXY first, then HTTP_PROXY
        settings
            .get("env")?
            .get("HTTPS_PROXY")
            .or_else(|| settings.get("env")?.get("HTTP_PROXY"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    fn fetch_api_usage(
        &self,
        api_base_url: &str,
        token: &str,
        timeout_secs: u64,
    ) -> Option<ApiUsageResponse> {
        let url = format!("{}/api/oauth/usage", api_base_url);
        let user_agent = Self::get_claude_code_version();

        let mut agent_builder = ureq::AgentBuilder::new();

        // Configure proxy from Claude settings if available
        if let Some(proxy_url) = Self::get_proxy_from_settings() {
            if let Ok(proxy) = ureq::Proxy::new(&proxy_url) {
                agent_builder = agent_builder.proxy(proxy);
            }
        }

        let agent = agent_builder.build();

        let response = match agent
            .get(&url)
            .set("Authorization", &format!("Bearer {}", token))
            .set("anthropic-beta", "oauth-2025-04-20")
            .set("User-Agent", &user_agent)
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .call()
        {
            Ok(resp) => resp,
            Err(e) => {
                log_debug("usage:api", &format!("HTTP request failed: {}", e));
                return None;
            }
        };

        let status = response.status();
        log_debug("usage:api", &format!("HTTP status: {}", status));

        if status != 200 {
            return None;
        }

        let body = match response.into_string() {
            Ok(b) => b,
            Err(e) => {
                log_debug("usage:api", &format!("failed to read body: {}", e));
                return None;
            }
        };
        log_debug("usage:api", &format!("response body: {}", body));

        match serde_json::from_str::<ApiUsageResponse>(&body) {
            Ok(parsed) => Some(parsed),
            Err(e) => {
                log_debug("usage:api", &format!("deserialization error: {}", e));
                None
            }
        }
    }
}

impl Segment for UsageSegment {
    fn collect(&self, _input: &InputData) -> Option<SegmentData> {
        let token = match credentials::get_oauth_token() {
            Some(t) => {
                log_debug("usage", "oauth token obtained");
                t
            }
            None => {
                log_debug("usage", "failed to get oauth token");
                return None;
            }
        };

        // Load config from file to get segment options
        let config = crate::config::Config::load().ok()?;
        let segment_config = config.segments.iter().find(|s| s.id == SegmentId::Usage);

        let api_base_url = segment_config
            .and_then(|sc| sc.options.get("api_base_url"))
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.anthropic.com");

        let cache_duration = segment_config
            .and_then(|sc| sc.options.get("cache_duration"))
            .and_then(|v| v.as_u64())
            .unwrap_or(300);

        let timeout = segment_config
            .and_then(|sc| sc.options.get("timeout"))
            .and_then(|v| v.as_u64())
            .unwrap_or(2);

        let cached_data = self.load_cache();
        let use_cached = cached_data
            .as_ref()
            .map(|cache| self.is_cache_valid(cache, cache_duration))
            .unwrap_or(false);

        let (five_hour_util, seven_day_util, resets_at) = if use_cached {
            let cache = cached_data.unwrap();
            log_debug("usage", "using cached data");
            (
                cache.five_hour_utilization,
                cache.seven_day_utilization,
                cache.resets_at,
            )
        } else {
            match self.fetch_api_usage(api_base_url, &token, timeout) {
                Some(response) => {
                    let seven_day_util = response.seven_day.as_ref().map(|s| s.utilization);
                    let resets_at = response
                        .seven_day
                        .as_ref()
                        .and_then(|s| s.resets_at.clone())
                        .or_else(|| response.five_hour.resets_at.clone());

                    log_debug(
                        "usage",
                        &format!(
                            "api result: 5h={}, 7d={:?}, resets_at={:?}",
                            response.five_hour.utilization, seven_day_util, resets_at
                        ),
                    );

                    let cache = ApiUsageCache {
                        five_hour_utilization: response.five_hour.utilization,
                        seven_day_utilization: seven_day_util,
                        resets_at: resets_at.clone(),
                        cached_at: Utc::now().to_rfc3339(),
                    };
                    self.save_cache(&cache);
                    (response.five_hour.utilization, seven_day_util, resets_at)
                }
                None => {
                    if let Some(cache) = cached_data {
                        log_debug("usage", "api failed, falling back to stale cache");
                        (
                            cache.five_hour_utilization,
                            cache.seven_day_utilization,
                            cache.resets_at,
                        )
                    } else {
                        log_debug("usage", "api failed and no cache available");
                        return None;
                    }
                }
            }
        };

        // Use seven_day utilization for the icon if available, otherwise five_hour
        let icon_util = seven_day_util.unwrap_or(five_hour_util);
        let dynamic_icon = Self::get_circle_icon(icon_util / 100.0);
        let five_hour_percent = five_hour_util.round() as u8;
        let primary = format!("{}%", five_hour_percent);

        let secondary = if resets_at.is_some() {
            format!("· {}", Self::format_reset_time(resets_at.as_deref()))
        } else {
            "· 5h only".to_string()
        };

        let mut metadata = HashMap::new();
        metadata.insert("dynamic_icon".to_string(), dynamic_icon);
        metadata.insert(
            "five_hour_utilization".to_string(),
            five_hour_util.to_string(),
        );
        metadata.insert(
            "seven_day_utilization".to_string(),
            seven_day_util
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
        );

        Some(SegmentData {
            primary,
            secondary,
            metadata,
        })
    }

    fn id(&self) -> SegmentId {
        SegmentId::Usage
    }
}
