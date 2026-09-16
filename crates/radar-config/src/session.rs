use std::path::{Path, PathBuf};

use crate::error::ConfigError;
use crate::load::load;
use crate::validate::{validate, Finding, Severity};
use crate::RadarConfig;

#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    Unconfigured,
    Invalid {
        path: PathBuf,
        findings: Vec<Finding>,
    },
    Ready {
        path: PathBuf,
        config: RadarConfig,
        warnings: Vec<Finding>,
    },
}

impl SessionState {
    pub fn from_dir(dir: &Path) -> Result<Self, ConfigError> {
        let config = load(dir)?;
        Ok(Self::from_config(dir.to_path_buf(), config))
    }

    pub fn from_config(path: PathBuf, config: RadarConfig) -> Self {
        let findings = validate(&config);
        let has_error = findings.iter().any(|finding| finding.severity == Severity::Error);
        if has_error {
            SessionState::Invalid { path, findings }
        } else {
            let warnings = findings
                .into_iter()
                .filter(|finding| finding.severity == Severity::Warning)
                .collect();
            SessionState::Ready {
                path,
                config,
                warnings,
            }
        }
    }
}
