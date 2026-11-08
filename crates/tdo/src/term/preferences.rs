//! Reload only local list/group preferences, never a different store or write policy.
use std::fs;
use std::path::PathBuf;
use twydo_core::config::Config;

pub struct PreferenceWatch {
    path: Option<PathBuf>,
    bytes: Option<Vec<u8>>,
    binding: Config,
}

impl PreferenceWatch {
    pub fn new(path: Option<PathBuf>, config: &Config) -> Self {
        let bytes = path.as_ref().and_then(|p| fs::read(p).ok());
        Self {
            path,
            bytes,
            binding: config.clone(),
        }
    }
    pub fn poll(&mut self, current: &Config) -> Result<Option<Config>, String> {
        let Some(path) = &self.path else {
            return Ok(None);
        };
        let bytes = fs::read(path).map_err(|e| format!("Cannot reload list preferences: {e}"))?;
        if self.bytes.as_ref() == Some(&bytes) {
            return Ok(None);
        }
        self.bytes = Some(bytes);
        let saved =
            Config::load(path).map_err(|e| format!("Cannot reload list preferences: {e}"))?;
        let same_vdir = fs::canonicalize(&saved.vdir)
            .ok()
            .zip(fs::canonicalize(&self.binding.vdir).ok())
            .is_some_and(|(a, b)| a == b);
        let same_state = std::path::absolute(saved.state_dir()).ok()
            == std::path::absolute(self.binding.state_dir()).ok();
        if !same_vdir
            || !same_state
            || saved.writes != self.binding.writes
            || saved.sync != self.binding.sync
        {
            return Err(
                "Configuration binding or write policy changed; restart Twydo to use it".into(),
            );
        }
        if saved.lists == current.lists && saved.groups == current.groups {
            return Ok(None);
        }
        let mut next = current.clone();
        next.lists = saved.lists;
        next.groups = saved.groups;
        Ok(Some(next))
    }
}
