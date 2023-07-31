use super::*;
pub fn home_dir() -> Result<PathBuf> {
    directories::UserDirs::new()
        .ok_or_else(|| eyre!("no user dirs"))
        .map(|user| user.home_dir().to_owned())
}

/// qpwgraph has a bug so the file must be persistent...
pub fn temp_path() -> Result<PathBuf> {
    home_dir().map(|parent| parent.join("qpwgraph-reaper-generated-session.qpwgraph"))
}

#[derive(Debug, Clone)]
pub struct ExistingDirectory(PathBuf);

impl AsRef<std::path::Path> for ExistingDirectory {
    fn as_ref(&self) -> &std::path::Path {
        self.0.as_ref()
    }
}

impl ExistingDirectory {
    pub fn check(dir: PathBuf) -> Result<Self> {
        make_sure_directory_exists(dir).map(ExistingDirectory)
    }
}

fn make_sure_directory_exists(dir: PathBuf) -> Result<PathBuf> {
    dir.try_exists()
        .wrap_err("what the hell")
        .and_then(|v| {
            v.then_some(dir.clone())
                .ok_or_else(|| eyre!("file does not exist"))
                .and_then(|dir| {
                    dir.is_dir()
                        .then_some(dir)
                        .ok_or_else(|| eyre!("this is not a directory"))
                })
        })
        .or_else(|message| {
            tracing::warn!(?message, ?dir, "making sure directory exists");
            std::fs::create_dir_all(&dir)
                .wrap_err("creating directory")
                .map(|_| dir)
        })
}
