//! Portable settings: `.nandi/settings.yaml`. Holds keyring references only;
//! resolving them is the caller's job (`services::secrets`).

use std::path::PathBuf;
use std::sync::Arc;

use super::fs_atomic::write_if_changed;
use super::git::GitIndex;
use super::{RepoError, RepoLayout, RepoResult, SingletonRepository};
use crate::services::domain::Settings;

#[derive(Clone)]
pub struct SettingsRepository {
    layout: Arc<RepoLayout>,
    git: GitIndex,
}

impl SettingsRepository {
    pub(super) fn new(layout: Arc<RepoLayout>, git: GitIndex) -> Self {
        Self { layout, git }
    }
}

impl SingletonRepository for SettingsRepository {
    type Entity = Settings;

    fn load(&self) -> RepoResult<Settings> {
        let path = self.path();
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(Settings::from_yaml(&content)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(RepoError::io(&path, e)),
        }
    }

    fn save(&self, settings: &Settings) -> RepoResult<PathBuf> {
        let path = self.path();
        if write_if_changed(&path, settings.to_yaml().as_bytes())
            .map_err(|e| RepoError::io(&path, e))?
        {
            self.git.stage_add(&path);
        }
        Ok(path)
    }

    fn path(&self) -> PathBuf {
        self.layout.settings_path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::settings::{SecretRef, Theme};
    use crate::services::repos::Repos;

    #[test]
    fn missing_file_is_defaults_and_refs_survive_a_save() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        assert_eq!(repos.settings.load().unwrap(), Settings::default());
        let mut settings = Settings::default();
        settings.set_theme(Theme::Dark);
        settings
            .set_secret_ref(
                "deepseek_api_key",
                SecretRef::parse("nandi.llm.deepseek").unwrap(),
            )
            .unwrap();
        repos.settings.save(&settings).unwrap();
        let text = std::fs::read_to_string(repos.settings.path()).unwrap();
        assert!(
            text.contains("deepseek_api_key: nandi.llm.deepseek"),
            "{text}"
        );
        assert_eq!(repos.settings.load().unwrap(), settings);
    }
}
