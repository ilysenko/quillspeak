use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use shared::AppConfig;
use tracing::{debug, warn};

use crate::command::AppCommand;

const SOCKET_DIR_NAME: &str = "myapp";
const SOCKET_FILE_NAME: &str = "command.sock";
const SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(50);
const SOCKET_READ_TIMEOUT: Duration = Duration::from_secs(2);
const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const COMMAND_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_EXTERNAL_TRIGGER_CLIENTS: usize = 16;
const USAGE: &str = "usage: myapp\n       myapp trigger <shortcut-id-or-name> <start|stop|toggle>";

pub fn usage() -> &'static str {
    USAGE
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalTriggerAction {
    Start,
    Stop,
    Toggle,
}

impl ExternalTriggerAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Toggle => "toggle",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "start" => Ok(Self::Start),
            "stop" => Ok(Self::Stop),
            "toggle" => Ok(Self::Toggle),
            other => Err(anyhow!("unsupported trigger action: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalTriggerRequest {
    pub shortcut: String,
    pub action: ExternalTriggerAction,
}

impl ExternalTriggerRequest {
    fn new(shortcut: impl Into<String>, action: ExternalTriggerAction) -> Result<Self> {
        let shortcut = shortcut.into();
        if shortcut.trim().is_empty() {
            bail!("shortcut cannot be empty");
        }
        if shortcut
            .chars()
            .any(|character| matches!(character, '\t' | '\n' | '\r'))
        {
            bail!("shortcut cannot contain tabs or newlines");
        }
        Ok(Self { shortcut, action })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalTriggerResponse {
    Accepted,
    Rejected(String),
}

impl ExternalTriggerResponse {
    pub fn accepted() -> Self {
        Self::Accepted
    }

    pub fn rejected(message: impl Into<String>) -> Self {
        Self::Rejected(message.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalTriggerInvocation {
    RunApp,
    ShowUsage,
    Send(ExternalTriggerRequest),
}

pub struct ExternalTriggerService {
    shutdown_requested: Arc<AtomicBool>,
    join_handle: Option<thread::JoinHandle<()>>,
    socket_path: PathBuf,
}

impl ExternalTriggerService {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>) -> Result<Self> {
        Self::spawn_for_path(command_socket_path()?, command_tx)
    }

    fn spawn_for_path(socket_path: PathBuf, command_tx: mpsc::Sender<AppCommand>) -> Result<Self> {
        Self::spawn_for_path_with_client_limit(
            socket_path,
            command_tx,
            MAX_EXTERNAL_TRIGGER_CLIENTS,
        )
    }

    fn spawn_for_path_with_client_limit(
        socket_path: PathBuf,
        command_tx: mpsc::Sender<AppCommand>,
        max_clients: usize,
    ) -> Result<Self> {
        anyhow::ensure!(
            max_clients > 0,
            "command socket client limit must be positive"
        );
        prepare_socket_path(&socket_path)?;
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("failed to bind command socket {}", socket_path.display()))?;

        let shutdown_requested = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown_requested);
        let worker_socket_path = socket_path.clone();
        let join_handle = thread::Builder::new()
            .name("myapp-external-trigger".to_string())
            .spawn(move || {
                command_socket_loop(listener, command_tx, worker_shutdown, max_clients);
                if let Err(error) = fs::remove_file(&worker_socket_path)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    warn!(
                        ?error,
                        socket_path = %worker_socket_path.display(),
                        "failed to remove command socket"
                    );
                }
            })
            .map_err(|error| anyhow!("failed to spawn command socket thread: {error}"))?;

        Ok(Self {
            shutdown_requested,
            join_handle: Some(join_handle),
            socket_path,
        })
    }

    pub fn shutdown(&mut self) {
        self.shutdown_requested.store(true, Ordering::Relaxed);
        if let Some(join_handle) = self.join_handle.take() {
            // Wake the blocking accept so the worker observes the shutdown flag.
            let _ = UnixStream::connect(&self.socket_path);
            if let Err(error) = join_handle.join() {
                warn!(
                    ?error,
                    socket_path = %self.socket_path.display(),
                    "command socket thread panicked during shutdown"
                );
            }
        }
    }
}

impl Drop for ExternalTriggerService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn parse_invocation<I, S>(args: I) -> Result<ExternalTriggerInvocation>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    if args.is_empty() {
        return Ok(ExternalTriggerInvocation::RunApp);
    }
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(ExternalTriggerInvocation::ShowUsage);
    }
    if args.len() == 3 && args[0] == "trigger" {
        let request =
            ExternalTriggerRequest::new(args[1].clone(), ExternalTriggerAction::parse(&args[2])?)?;
        return Ok(ExternalTriggerInvocation::Send(request));
    }
    bail!("{USAGE}");
}

pub fn send_trigger_request(request: &ExternalTriggerRequest) -> Result<()> {
    send_trigger_request_to_path(&command_socket_path()?, request)
}

pub fn resolve_shortcut_selector(config: &AppConfig, selector: &str) -> Result<String, String> {
    if let Some(shortcut) = config
        .shortcuts
        .iter()
        .find(|shortcut| shortcut.id == selector)
    {
        if shortcut.enabled {
            return Ok(shortcut.id.clone());
        }
        return Err(format!("shortcut '{selector}' is disabled"));
    }

    let matches = config
        .shortcuts
        .iter()
        .filter(|shortcut| shortcut.name == selector)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(format!("shortcut '{selector}' was not found")),
        [shortcut] if shortcut.enabled => Ok(shortcut.id.clone()),
        [shortcut] => Err(format!("shortcut '{}' is disabled", shortcut.name)),
        _ => Err(format!("shortcut name '{selector}' is ambiguous")),
    }
}

fn command_socket_path() -> Result<PathBuf> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| anyhow!("XDG_RUNTIME_DIR is not set; cannot locate MyApp command socket"))?;
    Ok(PathBuf::from(runtime_dir)
        .join(SOCKET_DIR_NAME)
        .join(SOCKET_FILE_NAME))
}

fn prepare_socket_path(socket_path: &Path) -> Result<()> {
    let parent = socket_path
        .parent()
        .ok_or_else(|| anyhow!("command socket path has no parent directory"))?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create command socket directory {}",
            parent.display()
        )
    })?;

    if !socket_path.exists() {
        return Ok(());
    }

    match UnixStream::connect(socket_path) {
        Ok(_) => Err(anyhow!(
            "command socket is already active at {}; is MyApp already running?",
            socket_path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
            fs::remove_file(socket_path).with_context(|| {
                format!(
                    "failed to remove stale command socket {}",
                    socket_path.display()
                )
            })?;
            Ok(())
        }
        Err(error) => Err(anyhow!(
            "command socket {} exists but its liveness probe failed ({error}); not removing it",
            socket_path.display()
        )),
    }
}

fn command_socket_loop(
    listener: UnixListener,
    command_tx: mpsc::Sender<AppCommand>,
    shutdown_requested: Arc<AtomicBool>,
    max_clients: usize,
) {
    let mut client_handlers = Vec::new();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if shutdown_requested.load(Ordering::Relaxed) {
                    break;
                }
                join_finished_client_handlers(&mut client_handlers);
                if client_handlers.len() >= max_clients {
                    reject_overloaded_connection(stream);
                    continue;
                }

                let connection_command_tx = command_tx.clone();
                match thread::Builder::new()
                    .name("myapp-external-trigger-client".to_string())
                    .spawn(move || handle_connection(stream, &connection_command_tx))
                {
                    Ok(join_handle) => client_handlers.push(join_handle),
                    Err(error) => warn!(?error, "failed to spawn command socket client handler"),
                }
            }
            Err(error) => {
                if shutdown_requested.load(Ordering::Relaxed) {
                    break;
                }
                warn!(?error, "failed to accept command socket connection");
                thread::sleep(SOCKET_POLL_INTERVAL);
            }
        }
    }
    join_all_client_handlers(client_handlers);
}

fn join_finished_client_handlers(client_handlers: &mut Vec<thread::JoinHandle<()>>) {
    let mut index = 0;
    while index < client_handlers.len() {
        if client_handlers[index].is_finished() {
            join_client_handler(client_handlers.swap_remove(index));
        } else {
            index += 1;
        }
    }
}

fn join_all_client_handlers(client_handlers: Vec<thread::JoinHandle<()>>) {
    for client_handler in client_handlers {
        join_client_handler(client_handler);
    }
}

fn join_client_handler(client_handler: thread::JoinHandle<()>) {
    if let Err(error) = client_handler.join() {
        warn!(?error, "command socket client handler panicked");
    }
}

fn reject_overloaded_connection(mut stream: UnixStream) {
    let response = ExternalTriggerResponse::rejected("too many active external trigger clients");
    if let Err(error) = write_response(&mut stream, &response) {
        debug!(?error, "failed to write command socket overload response");
    }
}

fn handle_connection(mut stream: UnixStream, command_tx: &mpsc::Sender<AppCommand>) {
    let response =
        match read_request(&stream).and_then(|request| dispatch_request(command_tx, request)) {
            Ok(response) => response,
            Err(error) => ExternalTriggerResponse::rejected(format!("{error:#}")),
        };

    if let Err(error) = write_response(&mut stream, &response) {
        debug!(?error, "failed to write command socket response");
    }
}

fn read_request(stream: &UnixStream) -> Result<ExternalTriggerRequest> {
    stream
        .set_read_timeout(Some(SOCKET_READ_TIMEOUT))
        .context("failed to configure command socket read timeout")?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .context("failed to clone command socket")?,
    );
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .context("failed to read command socket request")?;
    if bytes == 0 {
        bail!("empty command socket request");
    }
    parse_protocol_request(&line)
}

fn dispatch_request(
    command_tx: &mpsc::Sender<AppCommand>,
    request: ExternalTriggerRequest,
) -> Result<ExternalTriggerResponse> {
    let (response_tx, response_rx) = mpsc::channel();
    command_tx
        .send(AppCommand::ExternalTrigger {
            request,
            deadline: Instant::now() + COMMAND_RESPONSE_TIMEOUT,
            response_tx,
        })
        .context("failed to send external trigger command to app runtime")?;
    response_rx
        .recv_timeout(COMMAND_RESPONSE_TIMEOUT)
        .context("timed out waiting for app runtime to accept external trigger command")
}

fn write_response(stream: &mut UnixStream, response: &ExternalTriggerResponse) -> Result<()> {
    stream
        .set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))
        .context("failed to configure command socket write timeout")?;
    match response {
        ExternalTriggerResponse::Accepted => stream.write_all(b"ok\n")?,
        ExternalTriggerResponse::Rejected(message) => {
            stream.write_all(format!("error\t{}\n", sanitize_field(message)).as_bytes())?;
        }
    }
    Ok(())
}

fn send_trigger_request_to_path(
    socket_path: &Path,
    request: &ExternalTriggerRequest,
) -> Result<()> {
    let mut stream = UnixStream::connect(socket_path).with_context(|| {
        format!(
            "failed to connect to MyApp command socket {}; is MyApp running?",
            socket_path.display()
        )
    })?;
    stream
        .set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))
        .context("failed to configure command socket write timeout")?;
    stream
        .set_read_timeout(Some(COMMAND_RESPONSE_TIMEOUT))
        .context("failed to configure command socket read timeout")?;
    stream.write_all(serialize_protocol_request(request).as_bytes())?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .context("failed to finish command socket request")?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    let bytes = reader
        .read_line(&mut response)
        .context("failed to read MyApp command response")?;
    if bytes == 0 {
        bail!("MyApp closed the command socket without a response");
    }
    parse_protocol_response(&response)
}

fn serialize_protocol_request(request: &ExternalTriggerRequest) -> String {
    format!(
        "trigger\t{}\t{}\n",
        sanitize_field(&request.shortcut),
        request.action.as_str()
    )
}

fn parse_protocol_request(line: &str) -> Result<ExternalTriggerRequest> {
    let line = line.trim_end_matches(['\r', '\n']);
    let parts = line.split('\t').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "trigger" {
        bail!("invalid command socket request");
    }
    ExternalTriggerRequest::new(parts[1], ExternalTriggerAction::parse(parts[2])?)
}

fn parse_protocol_response(line: &str) -> Result<()> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line == "ok" {
        return Ok(());
    }
    if let Some(message) = line.strip_prefix("error\t") {
        bail!("{message}");
    }
    bail!("invalid MyApp command response");
}

fn sanitize_field(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\t' | '\n' | '\r' => ' ',
            other => other,
        })
        .collect()
}

#[cfg(test)]
fn temp_socket_path() -> PathBuf {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    env::temp_dir()
        .join(format!("myapp-external-trigger-test-{suffix}"))
        .join(SOCKET_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{DEFAULT_MODEL_ID, ShortcutProfile};

    #[test]
    fn parses_trigger_invocation() {
        assert_eq!(
            parse_invocation(["trigger", "Default", "start"]).unwrap(),
            ExternalTriggerInvocation::Send(ExternalTriggerRequest {
                shortcut: "Default".to_string(),
                action: ExternalTriggerAction::Start,
            })
        );
        assert_eq!(
            parse_invocation(["trigger", "default", "stop"]).unwrap(),
            ExternalTriggerInvocation::Send(ExternalTriggerRequest {
                shortcut: "default".to_string(),
                action: ExternalTriggerAction::Stop,
            })
        );
        assert_eq!(
            parse_invocation(["trigger", "Default", "toggle"]).unwrap(),
            ExternalTriggerInvocation::Send(ExternalTriggerRequest {
                shortcut: "Default".to_string(),
                action: ExternalTriggerAction::Toggle,
            })
        );
        assert_eq!(
            parse_invocation(Vec::<String>::new()).unwrap(),
            ExternalTriggerInvocation::RunApp
        );
        assert!(parse_invocation(["trigger", "Default"]).is_err());
        assert!(parse_invocation(["trigger", "Default", "toggle", "extra"]).is_err());
        assert!(parse_invocation(["trigger", "Default", "unknown"]).is_err());
    }

    #[test]
    fn protocol_round_trips_trigger_request_and_response() {
        let request =
            ExternalTriggerRequest::new("Default", ExternalTriggerAction::Toggle).unwrap();
        let encoded = serialize_protocol_request(&request);

        assert_eq!(parse_protocol_request(&encoded).unwrap(), request);
        assert!(parse_protocol_response("ok\n").is_ok());
        assert!(parse_protocol_response("error\tnope\n").is_err());
        assert!(parse_protocol_request("trigger\tDefault\tunknown\n").is_err());
    }

    #[test]
    fn resolves_shortcut_by_id_before_name() {
        let mut config = AppConfig::default();
        config.shortcuts.push(ShortcutProfile::new_profile(
            "Default".to_string(),
            "Other".to_string(),
            DEFAULT_MODEL_ID.to_string(),
        ));

        assert_eq!(
            resolve_shortcut_selector(&config, "Default").unwrap(),
            "Default"
        );
    }

    #[test]
    fn resolves_shortcut_by_unique_name() {
        let mut config = AppConfig::default();
        config.shortcuts.push(ShortcutProfile::new_profile(
            "shortcut-2".to_string(),
            "To English".to_string(),
            DEFAULT_MODEL_ID.to_string(),
        ));

        assert_eq!(
            resolve_shortcut_selector(&config, "To English").unwrap(),
            "shortcut-2"
        );
    }

    #[test]
    fn rejects_duplicate_or_disabled_shortcuts() {
        let mut config = AppConfig::default();
        config.shortcuts.push(ShortcutProfile::new_profile(
            "shortcut-2".to_string(),
            "Duplicate".to_string(),
            DEFAULT_MODEL_ID.to_string(),
        ));
        config.shortcuts.push(ShortcutProfile::new_profile(
            "shortcut-3".to_string(),
            "Duplicate".to_string(),
            DEFAULT_MODEL_ID.to_string(),
        ));
        config.shortcuts[0].enabled = false;

        assert!(resolve_shortcut_selector(&config, "Duplicate").is_err());
        assert!(resolve_shortcut_selector(&config, "default").is_err());
        assert!(resolve_shortcut_selector(&config, "Missing").is_err());
    }

    #[test]
    fn disabled_id_match_blocks_name_fallback() {
        let mut config = AppConfig::default();
        config.shortcuts.push(ShortcutProfile::new_profile(
            "legacy".to_string(),
            "Spare".to_string(),
            DEFAULT_MODEL_ID.to_string(),
        ));
        config.shortcuts.push(ShortcutProfile::new_profile(
            "shortcut-2".to_string(),
            "legacy".to_string(),
            DEFAULT_MODEL_ID.to_string(),
        ));
        let legacy_index = config
            .shortcuts
            .iter()
            .position(|shortcut| shortcut.id == "legacy")
            .expect("legacy shortcut should exist");
        config.shortcuts[legacy_index].enabled = false;

        // Id resolution wins even when disabled; the enabled shortcut whose
        // NAME is "legacy" must not be reached through name fallback.
        let error = resolve_shortcut_selector(&config, "legacy")
            .expect_err("disabled id match should not fall back to name resolution");
        assert!(error.contains("disabled"), "unexpected error: {error}");
    }

    #[test]
    fn socket_service_dispatches_valid_command_and_returns_ack() {
        let socket_path = temp_socket_path();
        let (command_tx, command_rx) = mpsc::channel();
        let mut service =
            ExternalTriggerService::spawn_for_path(socket_path.clone(), command_tx).unwrap();

        let request = ExternalTriggerRequest::new("Default", ExternalTriggerAction::Start).unwrap();
        let client = thread::spawn({
            let socket_path = socket_path.clone();
            let request = request.clone();
            move || send_trigger_request_to_path(&socket_path, &request)
        });

        let command = command_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("server should dispatch command");
        match command {
            AppCommand::ExternalTrigger {
                request: received,
                response_tx,
                ..
            } => {
                assert_eq!(received, request);
                response_tx
                    .send(ExternalTriggerResponse::accepted())
                    .expect("response should be sent");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        client
            .join()
            .expect("client thread should not panic")
            .expect("client should receive ok response");
        service.shutdown();
    }

    #[test]
    fn socket_service_dispatches_next_command_while_first_waits_for_ack() {
        let socket_path = temp_socket_path();
        let (command_tx, command_rx) = mpsc::channel();
        let mut service =
            ExternalTriggerService::spawn_for_path(socket_path.clone(), command_tx).unwrap();

        let first_request =
            ExternalTriggerRequest::new("Default", ExternalTriggerAction::Start).unwrap();
        let first_client = thread::spawn({
            let socket_path = socket_path.clone();
            let request = first_request.clone();
            move || send_trigger_request_to_path(&socket_path, &request)
        });

        let first_response_tx = match command_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first command should be dispatched")
        {
            AppCommand::ExternalTrigger {
                request: received,
                response_tx,
                ..
            } => {
                assert_eq!(received, first_request);
                response_tx
            }
            other => panic!("unexpected command: {other:?}"),
        };

        let second_request =
            ExternalTriggerRequest::new("Default", ExternalTriggerAction::Stop).unwrap();
        let second_client = thread::spawn({
            let socket_path = socket_path.clone();
            let request = second_request.clone();
            move || send_trigger_request_to_path(&socket_path, &request)
        });

        let second_response_tx = match command_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second command should be dispatched before first ack is sent")
        {
            AppCommand::ExternalTrigger {
                request: received,
                response_tx,
                ..
            } => {
                assert_eq!(received, second_request);
                response_tx
            }
            other => panic!("unexpected command: {other:?}"),
        };

        second_response_tx
            .send(ExternalTriggerResponse::accepted())
            .expect("second response should be sent");
        first_response_tx
            .send(ExternalTriggerResponse::accepted())
            .expect("first response should be sent");

        second_client
            .join()
            .expect("second client thread should not panic")
            .expect("second client should receive ok response");
        first_client
            .join()
            .expect("first client thread should not panic")
            .expect("first client should receive ok response");
        service.shutdown();
    }

    #[test]
    fn socket_service_rejects_connections_over_client_limit() {
        let socket_path = temp_socket_path();
        let (command_tx, command_rx) = mpsc::channel();
        let mut service = ExternalTriggerService::spawn_for_path_with_client_limit(
            socket_path.clone(),
            command_tx,
            1,
        )
        .unwrap();

        let first_request =
            ExternalTriggerRequest::new("Default", ExternalTriggerAction::Start).unwrap();
        let first_client = thread::spawn({
            let socket_path = socket_path.clone();
            let request = first_request.clone();
            move || send_trigger_request_to_path(&socket_path, &request)
        });

        let first_response_tx = match command_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first command should be dispatched")
        {
            AppCommand::ExternalTrigger {
                request: received,
                response_tx,
                ..
            } => {
                assert_eq!(received, first_request);
                response_tx
            }
            other => panic!("unexpected command: {other:?}"),
        };

        let second_request =
            ExternalTriggerRequest::new("Default", ExternalTriggerAction::Stop).unwrap();
        let error = send_trigger_request_to_path(&socket_path, &second_request)
            .expect_err("second client should be rejected while first is active");
        assert!(
            error
                .to_string()
                .contains("too many active external trigger clients")
        );
        assert!(
            command_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "over-limit connection should not dispatch an app command"
        );

        first_response_tx
            .send(ExternalTriggerResponse::accepted())
            .expect("first response should be sent");
        first_client
            .join()
            .expect("first client thread should not panic")
            .expect("first client should receive ok response");
        service.shutdown();
    }
}
