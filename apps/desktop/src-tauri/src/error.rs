use std::{error::Error, fmt, io};

#[derive(Debug)]
pub enum BootstrapError {
    AppDataDirectory(tauri::Error),
    CreateDirectories(io::Error),
    InitializeLogging(Box<dyn Error + Send + Sync>),
}

impl fmt::Display for BootstrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AppDataDirectory(_) => {
                formatter.write_str("failed to resolve the app data directory")
            }
            Self::CreateDirectories(_) => {
                formatter.write_str("failed to create runtime directories")
            }
            Self::InitializeLogging(_) => {
                formatter.write_str("failed to initialize application logging")
            }
        }
    }
}

impl Error for BootstrapError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::AppDataDirectory(error) => Some(error),
            Self::CreateDirectories(error) => Some(error),
            Self::InitializeLogging(error) => Some(error.as_ref()),
        }
    }
}
