//! Update checking against GitHub Releases.
//!
//! The repository is private today, so a token may be needed to read the release list; the
//! same code works unauthenticated once it is public. Nothing is downloaded or executed
//! automatically — a check only reports that a newer release exists.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

/// Where releases live.
pub const REPO: &str = "Zektopic/reIDM";

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    /// The version without a leading `v`, e.g. `0.2.0`.
    pub fn version(&self) -> &str {
        self.tag_name.strip_prefix('v').unwrap_or(&self.tag_name)
    }

    /// The downloadable asset for a platform, if the release carries one.
    pub fn asset_for(&self, platform: &str) -> Option<&Asset> {
        let suffix = match platform {
            "windows" => ".exe",
            "macos" => ".dmg",
            "linux" => ".deb",
            _ => return None,
        };
        self.assets.iter().find(|a| a.name.ends_with(suffix))
    }
}

/// `true` when `candidate` is a strictly newer `x.y.z` than `current`.
///
/// Components are compared numerically, so `0.10.0` is correctly newer than `0.9.0`.
/// Unparsable components are treated as 0, and any pre-release suffix is ignored.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parts(candidate), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// Look for a newer release. Returns `Ok(None)` when this build is already the newest.
pub fn check(repo: &str, current: &str, token: Option<&str>) -> Result<Option<Release>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("building the HTTP client")?;

    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let mut req = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", concat!("reIDM/", env!("CARGO_PKG_VERSION")));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }

    let resp = req.send().context("contacting api.github.com")?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        // Either no releases yet, or a private repo without a usable token: GitHub answers
        // 404 rather than 403 so as not to leak that the repository exists.
        anyhow::bail!("no published release found for {repo} (a private repository needs a token)");
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("GitHub refused the request ({status}); for a private repository set REIDM_GH_TOKEN");
    }
    let rel: Release = resp.error_for_status().context("GitHub returned an error")?.json().context("reading the release")?;

    Ok(is_newer(rel.version(), current).then_some(rel))
}

/// A token from the environment, if one is set. Used only to read a private repository's
/// releases; it is never written anywhere or sent to anyone but api.github.com.
pub fn token_from_env() -> Option<String> {
    for k in ["REIDM_GH_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(v) = std::env::var(k) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.3.0", "0.2.0"));
        assert!(is_newer("v0.2.1", "0.2.0"));
        assert!(is_newer("0.10.0", "0.9.0"), "numeric, not lexicographic");
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("v0.2.0-beta.1", "0.2.0"), "a pre-release of the same version is not newer");
    }

    #[test]
    fn asset_picking() {
        let r = Release {
            tag_name: "v9.9.9".into(),
            html_url: "x".into(),
            name: String::new(),
            body: String::new(),
            assets: vec![
                Asset { name: "reIDM-setup-9.9.9.exe".into(), browser_download_url: "a".into(), size: 1 },
                Asset { name: "reIDM-9.9.9-universal.dmg".into(), browser_download_url: "b".into(), size: 2 },
            ],
        };
        assert_eq!(r.version(), "9.9.9");
        assert_eq!(r.asset_for("windows").unwrap().browser_download_url, "a");
        assert_eq!(r.asset_for("macos").unwrap().browser_download_url, "b");
        assert!(r.asset_for("linux").is_none());
    }
}
