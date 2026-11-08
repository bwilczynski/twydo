//! Collection management. Archive preferences are local configuration, not CalDAV metadata.

pub mod backup;
pub mod helper;
pub mod operation;
pub mod prefs;
mod remote;
pub use remote::{ListRequest, ListResult};

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::compat::Matrix;
use crate::config::Config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListState {
    pub id: String,
    pub name: String,
    pub color: Option<(u8, u8, u8)>,
    pub archived: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ListError {
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub struct ListService {
    config: Config,
    config_path: Option<PathBuf>,
    config_bytes: Vec<u8>,
    matrix: Matrix,
}

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub(crate) fn plain_id(id: &str) -> bool {
    !id.starts_with('.') && crate::retired::plain(id)
}

impl ListService {
    pub fn open(cfg: &Config, config_path: Option<&Path>) -> io::Result<Self> {
        Self::open_with(cfg, config_path, Matrix::embedded().clone())
    }

    pub fn open_with(cfg: &Config, config_path: Option<&Path>, matrix: Matrix) -> io::Result<Self> {
        Ok(Self {
            config: cfg.clone(),
            config_path: config_path.map(Path::to_path_buf),
            config_bytes: config_path.map(fs::read).transpose()?.unwrap_or_default(),
            matrix,
        })
    }

    pub fn state(&self, id: &str) -> Result<ListState, ListError> {
        if !plain_id(id) {
            return Err(invalid("invalid list identity").into());
        }
        let path = self.config.vdir.join(id);
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_dir() || meta.is_symlink() {
            return Err(invalid("list must be a plain directory").into());
        }
        let name = fs::read_to_string(path.join("displayname"))
            .ok()
            .filter(|n| !n.trim().is_empty())
            .map(|n| n.trim().to_string())
            .unwrap_or_else(|| id.to_string());
        let color = fs::read_to_string(path.join("color"))
            .ok()
            .and_then(|s| crate::vdir::parse_color(&s));
        let archived = if let Some(path) = &self.config_path {
            let current = Config::load(path).map_err(|e| invalid(e.to_string()))?;
            if fs::canonicalize(&current.vdir)? != fs::canonicalize(&self.config.vdir)? {
                return Err(invalid("saved configuration belongs to another vdir").into());
            }
            current.list_prefs(id).archived
        } else {
            self.config.list_prefs(id).archived
        };
        Ok(ListState {
            id: id.into(),
            name,
            color,
            archived,
        })
    }

    pub fn set_archived(
        &mut self,
        id: &str,
        archived: bool,
        expected: &ListState,
    ) -> Result<ListState, ListError> {
        if self.state(id)? != *expected {
            return Err(ListError::Refused(
                "list changed; reload and try again".into(),
            ));
        }
        let path = self.config_path.as_deref().ok_or_else(|| {
            ListError::Refused("local archiving needs a saved configuration for this vdir".into())
        })?;
        self.config =
            prefs::set_archived(path, &self.config.vdir, id, archived, &self.config_bytes)?;
        self.config_bytes = fs::read(path)?;
        self.state(id)
    }
}
