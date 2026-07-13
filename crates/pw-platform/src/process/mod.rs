use std::{
    collections::VecDeque,
    ffi::OsString,
    io::{self, Read},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct ProcessSpec {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub current_dir: Option<PathBuf>,
    pub output_capacity: usize,
}

impl ProcessSpec {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            args: Vec::new(),
            env: Vec::new(),
            current_dir: None,
            output_capacity: 64 * 1024,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_dropped: u64,
    pub stderr_dropped: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error("failed to spawn child: {0}")]
    Spawn(#[source] io::Error),
    #[error("process operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("process state lock is poisoned")]
    Poisoned,
}

#[derive(Default)]
struct BoundedBytes {
    bytes: VecDeque<u8>,
    dropped: u64,
    capacity: usize,
}

impl BoundedBytes {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            ..Self::default()
        }
    }
    fn append(&mut self, data: &[u8]) {
        for byte in data {
            if self.bytes.len() == self.capacity {
                self.bytes.pop_front();
                self.dropped = self.dropped.saturating_add(1);
            }
            if self.capacity > 0 {
                self.bytes.push_back(*byte);
            } else {
                self.dropped = self.dropped.saturating_add(1);
            }
        }
    }
}

struct State {
    child: Child,
    status: Option<ExitStatus>,
}

pub struct ProcessSupervisor {
    state: Mutex<State>,
    pid: u32,
    generation: u64,
    stdout: Arc<Mutex<BoundedBytes>>,
    stderr: Arc<Mutex<BoundedBytes>>,
}

impl ProcessSupervisor {
    /// # Errors
    /// Returns an error when the operating system rejects the spawn.
    pub fn spawn(spec: &ProcessSpec) -> Result<Self, SupervisorError> {
        let mut command = Command::new(&spec.executable);
        command
            .args(&spec.args)
            .envs(spec.env.iter().cloned())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &spec.current_dir {
            command.current_dir(dir);
        }
        let mut child = command.spawn().map_err(SupervisorError::Spawn)?;
        let stdout = Arc::new(Mutex::new(BoundedBytes::new(spec.output_capacity)));
        let stderr = Arc::new(Mutex::new(BoundedBytes::new(spec.output_capacity)));
        if let Some(pipe) = child.stdout.take() {
            spawn_drain(pipe, stdout.clone());
        }
        if let Some(pipe) = child.stderr.take() {
            spawn_drain(pipe, stderr.clone());
        }
        Ok(Self {
            pid: child.id(),
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            state: Mutex::new(State {
                child,
                status: None,
            }),
            stdout,
            stderr,
        })
    }

    #[must_use]
    pub const fn pid(&self) -> u32 {
        self.pid
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// # Errors
    /// Returns an error when child status cannot be queried.
    pub fn try_wait(&self) -> Result<Option<ExitStatus>, SupervisorError> {
        let mut state = self.lock_state()?;
        if state.status.is_none() {
            state.status = state.child.try_wait()?;
        }
        Ok(state.status)
    }

    /// # Errors
    /// Returns an error when child status cannot be queried.
    pub fn wait_for_exit(&self, timeout: Duration) -> Result<Option<ExitStatus>, SupervisorError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// # Errors
    /// Returns an error when child status cannot be queried.
    pub fn is_healthy(&self) -> Result<bool, SupervisorError> {
        Ok(self.try_wait()?.is_none())
    }

    /// # Errors
    /// Returns an error when the child cannot be stopped and reaped.
    pub fn stop(&self, grace: Duration) -> Result<(), SupervisorError> {
        let _ = self.stop_generation(self.generation, grace)?;
        Ok(())
    }

    /// # Errors
    /// Returns an error when the matching child cannot be stopped and reaped.
    pub fn stop_generation(
        &self,
        generation: u64,
        grace: Duration,
    ) -> Result<bool, SupervisorError> {
        if generation != self.generation {
            return Ok(false);
        }
        if self.wait_for_exit(grace)?.is_some() {
            return Ok(true);
        }
        let mut state = self.lock_state()?;
        if state.status.is_none() {
            match state.child.kill() {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::InvalidInput => {}
                Err(error) => return Err(error.into()),
            }
            state.status = Some(state.child.wait()?);
        }
        Ok(true)
    }

    /// # Errors
    /// Returns an error when stop or replacement spawn fails.
    pub fn restart(&self, spec: &ProcessSpec) -> Result<Self, SupervisorError> {
        self.stop(Duration::from_secs(5))?;
        Self::spawn(spec)
    }

    #[must_use]
    pub fn output(&self) -> ProcessOutput {
        let stdout = self
            .stdout
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stderr = self
            .stderr
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ProcessOutput {
            stdout: stdout.bytes.iter().copied().collect(),
            stderr: stderr.bytes.iter().copied().collect(),
            stdout_dropped: stdout.dropped,
            stderr_dropped: stderr.dropped,
        }
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, State>, SupervisorError> {
        self.state.lock().map_err(|_| SupervisorError::Poisoned)
    }
}

impl Drop for ProcessSupervisor {
    fn drop(&mut self) {
        let _ = self.stop(Duration::from_secs(5));
    }
}

fn spawn_drain(mut pipe: impl Read + Send + 'static, output: Arc<Mutex<BoundedBytes>>) {
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .append(&buffer[..read]),
            }
        }
    });
}
