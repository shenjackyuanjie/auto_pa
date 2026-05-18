// shell

use std::{fmt::Display, path::PathBuf, sync::OnceLock};

use anyhow::Result;
use tracing::event;
pub mod device;
pub mod model;

static HDC_EXECUTEABLE: OnceLock<PathBuf> = OnceLock::new();

fn get_hdc_path() -> &'static PathBuf {
    if HDC_EXECUTEABLE.get().is_none() {
        // from env
        if let Ok(path) = std::env::var("HDC_PATH") {
            HDC_EXECUTEABLE.set(PathBuf::from(path.clone())).unwrap();
            event!(
                tracing::Level::INFO,
                "Setting hdc path from env: {:?}",
                path
            );
        } else {
            // find it
            event!(
                tracing::Level::WARN,
                "Failed to find hdc executable, falling back to default path"
            );
            HDC_EXECUTEABLE.set(PathBuf::from("hdc")).unwrap();
        }
    }
    HDC_EXECUTEABLE.get().unwrap()
}

pub fn set_hdc_path(path: PathBuf) {
    event!(tracing::Level::INFO, "Setting hdc path to {:?}", path);
    HDC_EXECUTEABLE.set(path).unwrap();
}

pub async fn inner_shell(
    args: Vec<impl Into<String> + std::convert::AsRef<std::ffi::OsStr>>,
) -> Result<Output> {
    let mut process = tokio::process::Command::new(get_hdc_path());
    process.args(args);
    let output = process.output().await?;
    if !output.status.success() {
        return Err(anyhow::anyhow!("hdc failed: {:?}", output));
    }
    Ok(Output::new(output.stdout, output.stderr))
}

#[derive(Debug, Clone)]
pub struct Output {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Output {
    pub fn new(stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self { stdout, stderr }
    }

    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    pub fn stdout_to_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    pub fn stderr_to_string(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }
}

impl Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.stdout_to_string(),)
    }
}

// marcos
#[macro_export]
macro_rules! exec {
    ($($arg:expr),* $(,)?) => {
        $crate::inner_shell(vec![$($arg),*])
    };
}

#[macro_export]
macro_rules! exec_device {
    ($device:expr, $($arg:expr),* $(,)?) => {
        $crate::inner_shell(vec!["-t", $device, $($arg),*])
    };
}

// marcos
#[macro_export]
macro_rules! shell {
    ($($arg:expr),* $(,)?) => {
        $crate::inner_shell(vec!["shell", $($arg),*])
    };
}

#[macro_export]
macro_rules! shell_device {
    ($device:expr, $($arg:expr),* $(,)?) => {
        $crate::inner_shell(vec!["-t", $device, "shell", $($arg),*])
    };
}
