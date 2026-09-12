use std::{
    env,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use rationale_model::{KernelRequest, KernelResponse, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot},
    time,
};

use crate::{FrameDecodeError, checked_frame_length};

const DEFAULT_MAX_PAYLOAD: usize = 8 * 1024 * 1024;
const DEFAULT_QUEUE_CAPACITY: usize = 64;

/// Configuration for locating and supervising the OCaml proof worker.
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    worker_path: PathBuf,
    request_timeout: Duration,
    max_payload: usize,
    restart_limit: u8,
    restart_backoff: Duration,
    queue_capacity: usize,
}

impl WorkerConfig {
    /// Configure an explicit worker executable path.
    #[must_use]
    pub fn new(worker_path: impl Into<PathBuf>) -> Self {
        Self {
            worker_path: worker_path.into(),
            request_timeout: Duration::from_secs(2),
            max_payload: DEFAULT_MAX_PAYLOAD,
            restart_limit: 1,
            restart_backoff: Duration::from_millis(25),
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
        }
    }

    /// Discover the worker from `RATIONALE_KERNEL_WORKER` or beside the host binary.
    ///
    /// # Errors
    ///
    /// Returns a typed unavailable error when neither location names a file.
    pub fn discover() -> Result<Self, ProofUnavailable> {
        if let Some(path) = env::var_os("RATIONALE_KERNEL_WORKER") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Ok(Self::new(path));
            }
            return Err(ProofUnavailable::new(
                ProofUnavailableReason::WorkerNotFound { path },
            ));
        }

        let executable = env::current_exe().map_err(|error| {
            ProofUnavailable::new(ProofUnavailableReason::WorkerDiscoveryFailed {
                detail: error.to_string(),
            })
        })?;
        let path = executable
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(worker_file_name());
        if path.is_file() {
            Ok(Self::new(path))
        } else {
            Err(ProofUnavailable::new(
                ProofUnavailableReason::WorkerNotFound { path },
            ))
        }
    }

    /// Set the deadline applied to startup handshakes and proof requests.
    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Set the maximum encoded request or response frame size.
    #[must_use]
    pub const fn with_max_payload(mut self, max_payload: usize) -> Self {
        self.max_payload = max_payload;
        self
    }

    /// Set the number of restarts per request and the linear backoff base.
    #[must_use]
    pub const fn with_restart_policy(mut self, limit: u8, backoff: Duration) -> Self {
        self.restart_limit = limit;
        self.restart_backoff = backoff;
        self
    }

    /// Return the configured worker path.
    #[must_use]
    pub fn worker_path(&self) -> &Path {
        &self.worker_path
    }

    fn validate(&self) -> Result<(), ProofUnavailable> {
        if self.max_payload == 0 || self.max_payload > u32::MAX as usize {
            return Err(ProofUnavailable::new(
                ProofUnavailableReason::InvalidConfiguration {
                    detail: "maximum payload must fit in a non-zero uint32".to_owned(),
                },
            ));
        }
        if self.request_timeout.is_zero() {
            return Err(ProofUnavailable::new(
                ProofUnavailableReason::InvalidConfiguration {
                    detail: "request timeout must be non-zero".to_owned(),
                },
            ));
        }
        if self.queue_capacity == 0 {
            return Err(ProofUnavailable::new(
                ProofUnavailableReason::InvalidConfiguration {
                    detail: "worker queue capacity must be non-zero".to_owned(),
                },
            ));
        }
        Ok(())
    }
}

fn worker_file_name() -> &'static str {
    if cfg!(windows) {
        "rationale-kernel-worker.exe"
    } else {
        "rationale-kernel-worker"
    }
}

/// A failure to obtain an authoritative result from the proof kernel.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("proof unavailable: {reason}")]
pub struct ProofUnavailable {
    /// Machine-distinguishable reason the proof worker was unavailable.
    pub reason: ProofUnavailableReason,
}

impl ProofUnavailable {
    const fn new(reason: ProofUnavailableReason) -> Self {
        Self { reason }
    }
}

/// Operational reasons an authoritative proof could not be produced.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProofUnavailableReason {
    /// No worker executable exists at the resolved location.
    #[error("worker executable not found at {path}", path = .path.display())]
    WorkerNotFound {
        /// Resolved path that was checked.
        path: PathBuf,
    },
    /// The host process could not determine its companion-worker location.
    #[error("worker discovery failed: {detail}")]
    WorkerDiscoveryFailed {
        /// Redacted operating-system detail.
        detail: String,
    },
    /// A supervisor setting is invalid.
    #[error("invalid worker configuration: {detail}")]
    InvalidConfiguration {
        /// Configuration validation detail.
        detail: String,
    },
    /// The worker could not be started.
    #[error("worker startup failed: {detail}")]
    StartFailed {
        /// Redacted operating-system detail.
        detail: String,
    },
    /// The initial worker version negotiation failed.
    #[error("worker handshake failed: {detail}")]
    HandshakeFailed {
        /// Protocol failure detail.
        detail: String,
    },
    /// A request exceeded its configured deadline.
    #[error("worker request timed out")]
    TimedOut,
    /// The worker stream closed or failed during a request.
    #[error("worker crashed or disconnected: {detail}")]
    Crashed {
        /// Redacted stream detail.
        detail: String,
    },
    /// The worker returned an invalid or unexpected protocol message.
    #[error("worker protocol violation: {detail}")]
    ProtocolViolation {
        /// Validation detail.
        detail: String,
    },
    /// An encoded frame exceeds the configured bound.
    #[error("worker frame length {length} exceeds limit {limit}")]
    FrameTooLarge {
        /// Encoded frame length.
        length: usize,
        /// Configured frame bound.
        limit: usize,
    },
    /// The background supervisor is no longer accepting work.
    #[error("worker supervisor stopped")]
    SupervisorStopped,
}

/// Cancellation-safe asynchronous access to one long-lived OCaml proof worker.
#[derive(Clone, Debug)]
pub struct WorkerSupervisor {
    sender: mpsc::Sender<EvaluateCommand>,
    process_id: Arc<AtomicU32>,
}

impl WorkerSupervisor {
    /// Start the worker and validate its protocol version.
    ///
    /// # Errors
    ///
    /// Returns `ProofUnavailable` if configuration, startup, or handshake fails.
    pub async fn start(config: WorkerConfig) -> Result<Self, ProofUnavailable> {
        config.validate()?;
        let process_id = Arc::new(AtomicU32::new(0));
        let worker = Worker::start(&config, &process_id).await?;
        let (sender, receiver) = mpsc::channel(config.queue_capacity);
        tokio::spawn(run_supervisor(
            receiver,
            Some(worker),
            config,
            Arc::clone(&process_id),
        ));
        Ok(Self { sender, process_id })
    }

    /// Evaluate one bounded evidence request in the authoritative OCaml kernel.
    ///
    /// Dropping the returned future does not cancel the in-flight worker exchange;
    /// the supervisor drains it before processing the next queued request.
    ///
    /// # Errors
    ///
    /// Returns `ProofUnavailable` for operational or framing failures. Such failures
    /// are never converted to a `KernelResponse` verdict.
    pub async fn evaluate(
        &self,
        request: KernelRequest,
    ) -> Result<KernelResponse, ProofUnavailable> {
        let (respond_to, response) = oneshot::channel();
        self.sender
            .send(EvaluateCommand {
                request,
                respond_to,
            })
            .await
            .map_err(|_| ProofUnavailable::new(ProofUnavailableReason::SupervisorStopped))?;
        response
            .await
            .map_err(|_| ProofUnavailable::new(ProofUnavailableReason::SupervisorStopped))?
    }

    /// Return the current worker process identifier for operational diagnostics.
    #[must_use]
    pub fn worker_process_id(&self) -> Option<u32> {
        match self.process_id.load(Ordering::Acquire) {
            0 => None,
            process_id => Some(process_id),
        }
    }
}

struct EvaluateCommand {
    request: KernelRequest,
    respond_to: oneshot::Sender<Result<KernelResponse, ProofUnavailable>>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HostMessage<'a> {
    Hello { protocol_version: u16 },
    Evaluate { request: &'a KernelRequest },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerMessage {
    Ready { protocol_version: u16 },
    Result { response: KernelResponse },
    Failure { message: String },
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl Worker {
    async fn start(
        config: &WorkerConfig,
        process_id: &AtomicU32,
    ) -> Result<Self, ProofUnavailable> {
        let mut command = Command::new(&config.worker_path);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            ProofUnavailable::new(ProofUnavailableReason::StartFailed {
                detail: error.to_string(),
            })
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            ProofUnavailable::new(ProofUnavailableReason::StartFailed {
                detail: "worker stdin was not captured".to_owned(),
            })
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ProofUnavailable::new(ProofUnavailableReason::StartFailed {
                detail: "worker stdout was not captured".to_owned(),
            })
        })?;
        process_id.store(child.id().unwrap_or(0), Ordering::Release);
        let mut worker = Self {
            child,
            stdin,
            stdout,
        };
        let handshake = time::timeout(
            config.request_timeout,
            worker.exchange(
                &HostMessage::Hello {
                    protocol_version: PROTOCOL_VERSION,
                },
                config.max_payload,
            ),
        )
        .await;
        match handshake {
            Ok(Ok(WorkerMessage::Ready { protocol_version }))
                if protocol_version == PROTOCOL_VERSION =>
            {
                Ok(worker)
            }
            Ok(Ok(WorkerMessage::Ready { protocol_version })) => {
                worker.terminate(process_id).await;
                Err(ProofUnavailable::new(
                    ProofUnavailableReason::HandshakeFailed {
                        detail: format!(
                            "expected version {PROTOCOL_VERSION}, received {protocol_version}"
                        ),
                    },
                ))
            }
            Ok(Ok(WorkerMessage::Failure { message })) => {
                worker.terminate(process_id).await;
                Err(ProofUnavailable::new(
                    ProofUnavailableReason::HandshakeFailed { detail: message },
                ))
            }
            Ok(Ok(WorkerMessage::Result { .. })) => {
                worker.terminate(process_id).await;
                Err(ProofUnavailable::new(
                    ProofUnavailableReason::HandshakeFailed {
                        detail: "worker returned a proof before readiness".to_owned(),
                    },
                ))
            }
            Ok(Err(error)) => {
                worker.terminate(process_id).await;
                Err(ProofUnavailable::new(
                    ProofUnavailableReason::HandshakeFailed {
                        detail: error.to_string(),
                    },
                ))
            }
            Err(_) => {
                worker.terminate(process_id).await;
                Err(ProofUnavailable::new(
                    ProofUnavailableReason::HandshakeFailed {
                        detail: "worker readiness timed out".to_owned(),
                    },
                ))
            }
        }
    }

    async fn exchange(
        &mut self,
        message: &HostMessage<'_>,
        max_payload: usize,
    ) -> Result<WorkerMessage, ProofUnavailable> {
        let payload = serde_json::to_vec(message).map_err(|error| {
            ProofUnavailable::new(ProofUnavailableReason::ProtocolViolation {
                detail: format!("could not encode worker message: {error}"),
            })
        })?;
        if payload.len() > max_payload {
            return Err(ProofUnavailable::new(
                ProofUnavailableReason::FrameTooLarge {
                    length: payload.len(),
                    limit: max_payload,
                },
            ));
        }
        let length = u32::try_from(payload.len())
            .expect("validated worker payload must fit in a uint32")
            .to_be_bytes();
        self.stdin
            .write_all(&length)
            .await
            .map_err(|error| crashed(&error))?;
        self.stdin
            .write_all(&payload)
            .await
            .map_err(|error| crashed(&error))?;
        self.stdin.flush().await.map_err(|error| crashed(&error))?;

        let mut header = [0_u8; 4];
        self.stdout
            .read_exact(&mut header)
            .await
            .map_err(|error| crashed(&error))?;
        let response_length = checked_frame_length(header, max_payload).map_err(|error| {
            let FrameDecodeError::FrameTooLarge { length, limit } = error else {
                unreachable!("frame header validation only returns size failures")
            };
            ProofUnavailable::new(ProofUnavailableReason::FrameTooLarge { length, limit })
        })?;
        let mut response = vec![0_u8; response_length];
        self.stdout
            .read_exact(&mut response)
            .await
            .map_err(|error| crashed(&error))?;
        serde_json::from_slice(&response).map_err(|error| {
            ProofUnavailable::new(ProofUnavailableReason::ProtocolViolation {
                detail: format!("invalid worker response: {error}"),
            })
        })
    }

    async fn terminate(&mut self, process_id: &AtomicU32) {
        let _ = self.child.kill().await;
        process_id.store(0, Ordering::Release);
    }
}

fn crashed(error: &std::io::Error) -> ProofUnavailable {
    ProofUnavailable::new(ProofUnavailableReason::Crashed {
        detail: error.to_string(),
    })
}

async fn run_supervisor(
    mut receiver: mpsc::Receiver<EvaluateCommand>,
    mut worker: Option<Worker>,
    config: WorkerConfig,
    process_id: Arc<AtomicU32>,
) {
    while let Some(command) = receiver.recv().await {
        let result =
            evaluate_with_restart(&mut worker, &config, &process_id, &command.request).await;
        let _ = command.respond_to.send(result);
    }
    if let Some(mut worker) = worker {
        worker.terminate(&process_id).await;
    }
}

async fn evaluate_with_restart(
    worker: &mut Option<Worker>,
    config: &WorkerConfig,
    process_id: &AtomicU32,
    request: &KernelRequest,
) -> Result<KernelResponse, ProofUnavailable> {
    let mut attempt = 0_u8;
    loop {
        if worker.is_none() {
            match Worker::start(config, process_id).await {
                Ok(started) => *worker = Some(started),
                Err(error) if attempt >= config.restart_limit => return Err(error),
                Err(_) => {
                    attempt += 1;
                    time::sleep(config.restart_backoff.saturating_mul(u32::from(attempt))).await;
                    continue;
                }
            }
        }

        let message = HostMessage::Evaluate { request };
        let exchange = worker
            .as_mut()
            .expect("worker is initialized before exchange")
            .exchange(&message, config.max_payload);
        let result = match time::timeout(config.request_timeout, exchange).await {
            Ok(result) => result,
            Err(_) => Err(ProofUnavailable::new(ProofUnavailableReason::TimedOut)),
        };

        match result {
            Ok(WorkerMessage::Result { response }) => return Ok(response),
            Ok(WorkerMessage::Failure { message }) => {
                return Err(ProofUnavailable::new(
                    ProofUnavailableReason::ProtocolViolation { detail: message },
                ));
            }
            Ok(WorkerMessage::Ready { .. }) => {
                return Err(ProofUnavailable::new(
                    ProofUnavailableReason::ProtocolViolation {
                        detail: "worker returned readiness during evaluation".to_owned(),
                    },
                ));
            }
            Err(
                error @ ProofUnavailable {
                    reason: ProofUnavailableReason::FrameTooLarge { .. },
                },
            ) => {
                if let Some(active) = worker.as_mut() {
                    active.terminate(process_id).await;
                }
                *worker = None;
                return Err(error);
            }
            Err(error) => {
                if let Some(active) = worker.as_mut() {
                    active.terminate(process_id).await;
                }
                *worker = None;
                if attempt >= config.restart_limit {
                    return Err(error);
                }
                attempt += 1;
                time::sleep(config.restart_backoff.saturating_mul(u32::from(attempt))).await;
            }
        }
    }
}
