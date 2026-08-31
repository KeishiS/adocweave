use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener};
#[cfg(test)]
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use adocweave_core::CancellationToken;
use adocweave_core::output::diagnostics::{self, Diagnostic};
use serde::Serialize;

mod dependency;
mod http;

pub(crate) use dependency::{Dependency, DependencyKind, Fingerprint};
use http::{HttpSnapshot, HttpWorkers};

const ACCEPT_RETRY_INTERVAL: Duration = Duration::from_millis(20);
const DEPENDENCY_POLL_INTERVAL: Duration = Duration::from_millis(200);
const DEPENDENCY_FORCE_HASH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum PreviewDiagnostic {
    Analysis(Diagnostic),
    Include {
        code: String,
        message: String,
        target: String,
    },
    Build {
        code: &'static str,
        message: String,
    },
}

impl PreviewDiagnostic {
    pub(crate) fn analysis(diagnostics: &[Diagnostic]) -> Vec<Self> {
        let mut diagnostics = diagnostics.to_vec();
        diagnostics::sort_diagnostics(&mut diagnostics);
        diagnostics.into_iter().map(Self::Analysis).collect()
    }

    pub(crate) fn include(code: &str, message: String, target: &str) -> Self {
        Self::Include {
            code: code.to_owned(),
            message,
            target: target.to_owned(),
        }
    }

    fn build(message: &str) -> Self {
        Self::Build {
            code: "preview-build",
            message: message.to_owned(),
        }
    }
}

pub(crate) fn serialize_diagnostics(diagnostics: &[PreviewDiagnostic]) -> String {
    serde_json::to_string(diagnostics).expect("preview diagnostics are serializable")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Options {
    pub bind: IpAddr,
    pub port: u16,
    pub debounce: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Build {
    pub html: String,
    pub diagnostics: String,
    dependencies: BTreeMap<Dependency, Fingerprint>,
    style_origins: BTreeSet<String>,
    retain_previous_dependencies: bool,
}

impl Build {
    pub fn new(
        html: String,
        diagnostics: String,
        dependencies: BTreeMap<Dependency, Fingerprint>,
    ) -> Self {
        Self {
            html,
            diagnostics,
            dependencies,
            style_origins: BTreeSet::new(),
            retain_previous_dependencies: false,
        }
    }

    pub fn failure(message: String, dependencies: BTreeMap<Dependency, Fingerprint>) -> Self {
        let diagnostics = failure_diagnostics(&message);
        Self {
            retain_previous_dependencies: true,
            ..Self::new(error_document(&message), diagnostics, dependencies)
        }
    }

    pub fn with_style_origins(mut self, origins: BTreeSet<String>) -> Self {
        self.style_origins = origins;
        self
    }

    #[cfg(test)]
    pub(crate) fn dependency_count(&self) -> usize {
        self.dependencies.len()
    }

    #[cfg(test)]
    pub(crate) fn has_dependency(&self, path: &std::path::Path) -> bool {
        self.dependencies
            .keys()
            .any(|dependency| dependency.path() == path)
    }

    fn changed(
        &mut self,
        snapshot: &mut impl FnMut(&[Dependency]) -> BTreeMap<Dependency, Fingerprint>,
    ) -> bool {
        refresh_dependencies(&mut self.dependencies, snapshot)
    }

    fn retain_dependencies_from(&mut self, previous: &BTreeMap<Dependency, Fingerprint>) {
        if self.retain_previous_dependencies {
            for (dependency, fingerprint) in previous {
                self.dependencies
                    .entry(dependency.clone())
                    .or_insert_with(|| fingerprint.clone());
            }
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Bind {
        address: SocketAddr,
        source: io::Error,
    },
    Io(io::Error),
    Build(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind { address, source } => {
                write!(
                    formatter,
                    "could not bind preview server to {address}: {source}"
                )
            }
            Self::Io(source) => source.fmt(formatter),
            Self::Build(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone)]
struct State {
    http: Arc<HttpSnapshot>,
    dependencies: BTreeMap<Dependency, Fingerprint>,
}

impl State {
    fn from_build(generation: u64, build: Build) -> Self {
        Self {
            http: Arc::new(HttpSnapshot::new(
                generation,
                build.html,
                build.diagnostics,
                build.style_origins,
            )),
            dependencies: build.dependencies,
        }
    }

    fn changed(
        &mut self,
        _force_hash: bool,
        snapshot: &mut impl FnMut(&[Dependency]) -> BTreeMap<Dependency, Fingerprint>,
    ) -> bool {
        refresh_dependencies(&mut self.dependencies, snapshot)
    }

    fn refresh(
        &mut self,
        snapshot: &mut impl FnMut(&[Dependency]) -> BTreeMap<Dependency, Fingerprint>,
    ) {
        let _ = refresh_dependencies(&mut self.dependencies, snapshot);
    }

    fn replace_failure(
        &mut self,
        generation: u64,
        message: &str,
        snapshot: &mut impl FnMut(&[Dependency]) -> BTreeMap<Dependency, Fingerprint>,
    ) {
        self.http = Arc::new(self.http.failure(
            generation,
            error_document(message),
            failure_diagnostics(message),
        ));
        self.refresh(snapshot);
    }
}

fn refresh_dependencies(
    dependencies: &mut BTreeMap<Dependency, Fingerprint>,
    snapshot: &mut impl FnMut(&[Dependency]) -> BTreeMap<Dependency, Fingerprint>,
) -> bool {
    let keys = dependencies.keys().cloned().collect::<Vec<_>>();
    let mut latest = snapshot(&keys);
    let changed = keys.iter().any(|dependency| {
        latest
            .get(dependency)
            .is_none_or(|fingerprint| dependencies.get(dependency) != Some(fingerprint))
    });
    for dependency in keys {
        let fingerprint = latest
            .remove(&dependency)
            .unwrap_or_else(|| Fingerprint::unavailable("snapshot-missing"));
        dependencies.insert(dependency, fingerprint);
    }
    changed
}

#[derive(Clone, Debug)]
struct DependencyPoll {
    next_poll: Instant,
    next_force_hash: Instant,
}

#[derive(Clone, Debug)]
struct ChangeDebounce {
    delay: Duration,
    changed_at: Option<Instant>,
}

impl ChangeDebounce {
    fn new(delay: Duration) -> Self {
        Self {
            delay,
            changed_at: None,
        }
    }

    fn observe(&mut self, now: Instant) {
        self.changed_at.get_or_insert(now);
    }

    fn ready(&self, now: Instant) -> bool {
        self.changed_at
            .is_some_and(|start| now.saturating_duration_since(start) >= self.delay)
    }

    fn restart(&mut self, now: Instant) {
        self.changed_at = Some(now);
    }

    fn clear(&mut self) {
        self.changed_at = None;
    }
}

impl DependencyPoll {
    fn new(now: Instant) -> Self {
        Self {
            next_poll: now,
            next_force_hash: now + DEPENDENCY_FORCE_HASH_INTERVAL,
        }
    }

    fn mode(&mut self, now: Instant) -> Option<bool> {
        if now < self.next_poll {
            return None;
        }
        self.next_poll = now + DEPENDENCY_POLL_INTERVAL;
        let force_hash = now >= self.next_force_hash;
        if force_hash {
            self.next_force_hash = now + DEPENDENCY_FORCE_HASH_INTERVAL;
        }
        Some(force_hash)
    }

    fn reset(&mut self, now: Instant) {
        self.next_poll = now + DEPENDENCY_POLL_INTERVAL;
        self.next_force_hash = now + DEPENDENCY_FORCE_HASH_INTERVAL;
    }
}

fn run_build<T: Send>(
    cancellation: &CancellationToken,
    build: impl FnOnce(&CancellationToken) -> T + Send,
    mut monitor: impl FnMut() -> Result<(), Error>,
) -> Result<T, Error> {
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| build(cancellation));
        while !worker.is_finished() {
            monitor()?;
        }
        worker
            .join()
            .map_err(|_| Error::Build("preview build worker panicked".to_owned()))
    })
}

/// Runs the preview loop with cancellation at cooperative build-stage boundaries.
///
/// The build callback must observe its cancellation token between non-cooperative
/// stages. A stage which does not accept the token runs to completion.
/// The server waits for an active callback to finish before rebuilding or
/// shutting down; it does not impose a deadline on callbacks which ignore the
/// token.
pub fn run(
    options: Options,
    mut build: impl FnMut(&CancellationToken) -> Result<Build, String> + Send,
    mut snapshot: impl FnMut(&[Dependency]) -> BTreeMap<Dependency, Fingerprint>,
    shutdown: &AtomicBool,
) -> Result<(), Error> {
    let address = SocketAddr::new(options.bind, options.port);
    let listener = TcpListener::bind(address).map_err(|source| Error::Bind { address, source })?;
    listener.set_nonblocking(true).map_err(Error::Io)?;
    let local = listener.local_addr().map_err(Error::Io)?;
    eprintln!("AdocWeave preview: http://{local}/");

    let cancellation = CancellationToken::new();
    let first = run_build(
        &cancellation,
        |cancellation| build(cancellation),
        || {
            if shutdown.load(Ordering::Acquire) {
                cancellation.cancel();
            }
            std::thread::sleep(ACCEPT_RETRY_INTERVAL);
            Ok(())
        },
    )?;
    if shutdown.load(Ordering::Acquire) {
        return Ok(());
    }
    let first = first.map_err(Error::Build)?;
    let mut state = State::from_build(1, first);
    let http_workers = HttpWorkers::new().map_err(Error::Io)?;
    let mut debounce = ChangeDebounce::new(options.debounce);
    let mut dependency_poll = DependencyPoll::new(Instant::now());
    while !shutdown.load(Ordering::Acquire) {
        let now = Instant::now();
        if dependency_poll
            .mode(now)
            .is_some_and(|force_hash| state.changed(force_hash, &mut snapshot))
        {
            debounce.observe(now);
        }
        if debounce.ready(now) {
            state.refresh(&mut snapshot);
            dependency_poll.reset(Instant::now());
            let cancellation = CancellationToken::new();
            let mut superseded = false;
            let result = run_build(
                &cancellation,
                |cancellation| build(cancellation),
                || {
                    let dependency_changed = dependency_poll
                        .mode(Instant::now())
                        .is_some_and(|force_hash| state.changed(force_hash, &mut snapshot));
                    if shutdown.load(Ordering::Acquire) || dependency_changed {
                        cancellation.cancel();
                        superseded = true;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => {
                            http_workers.dispatch(stream, &state.http, local);
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            std::thread::sleep(ACCEPT_RETRY_INTERVAL);
                        }
                        Err(error) => return Err(Error::Io(error)),
                    }
                    Ok(())
                },
            )?;
            if state.changed(true, &mut snapshot) {
                superseded = true;
            }
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            if superseded {
                state.refresh(&mut snapshot);
                dependency_poll.reset(Instant::now());
                debounce.restart(Instant::now());
                continue;
            }
            let next_generation = state.http.generation().saturating_add(1);
            match result {
                Ok(mut next) => {
                    next.retain_dependencies_from(&state.dependencies);
                    if next.changed(&mut snapshot) {
                        state.refresh(&mut snapshot);
                        dependency_poll.reset(Instant::now());
                        debounce.restart(Instant::now());
                        continue;
                    }
                    state = State::from_build(next_generation, next);
                }
                Err(message) => {
                    state.replace_failure(next_generation, &message, &mut snapshot);
                }
            }
            debounce.clear();
        }

        match listener.accept() {
            Ok((stream, _)) => http_workers.dispatch(stream, &state.http, local),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_RETRY_INTERVAL);
            }
            Err(error) => return Err(Error::Io(error)),
        }
    }
    Ok(())
}

fn failure_diagnostics(message: &str) -> String {
    serialize_diagnostics(&[PreviewDiagnostic::build(message)])
}

fn error_document(message: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Preview error</title></head><body><h1>Preview error</h1><pre>{}</pre></body></html>\n",
        escape_html(message)
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&#34;")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpStream};
    use std::sync::atomic::AtomicU64;

    use adocweave_core::CancellationCheck;

    use super::*;

    fn snapshot_dependencies(dependencies: &[Dependency]) -> BTreeMap<Dependency, Fingerprint> {
        dependencies
            .iter()
            .cloned()
            .map(|dependency| {
                let fingerprint = match dependency.kind() {
                    DependencyKind::Contents | DependencyKind::ContentsNoSymlinks => {
                        fs::read(dependency.path()).map_or_else(
                            |error| Fingerprint::unavailable(&error.kind().to_string()),
                            |bytes| Fingerprint::from_loaded_bytes(&bytes),
                        )
                    }
                    DependencyKind::Existence => {
                        if dependency.path().is_file() {
                            Fingerprint::present()
                        } else {
                            Fingerprint::missing()
                        }
                    }
                };
                (dependency, fingerprint)
            })
            .collect()
    }

    fn snapshots(paths: impl IntoIterator<Item = PathBuf>) -> BTreeMap<Dependency, Fingerprint> {
        paths
            .into_iter()
            .map(|path| {
                let dependency = Dependency::contents(path);
                let fingerprint = snapshot_dependencies(std::slice::from_ref(&dependency))
                    .remove(&dependency)
                    .expect("dependency fingerprint");
                (dependency, fingerprint)
            })
            .collect()
    }

    fn raw_request(address: SocketAddr, request: &str) -> String {
        for _ in 0..100 {
            if let Ok(mut stream) = TcpStream::connect(address) {
                stream.write_all(request.as_bytes()).expect("request");
                let mut response = String::new();
                if stream.read_to_string(&mut response).is_ok() {
                    return response;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("preview server did not start");
    }

    fn request(address: SocketAddr, path: &str) -> String {
        raw_request(
            address,
            &format!("GET {path} HTTP/1.1\r\nHost: {address}\r\n\r\n"),
        )
    }

    #[test]
    fn error_page_escapes_input() {
        let page = error_document("</pre><script>alert(1)</script>");
        assert!(!page.contains("<script>alert"));
        assert!(page.contains("&lt;/pre&gt;"));
    }

    #[test]
    fn build_failures_keep_diagnostics_as_json() {
        let build = Build::failure("missing include".to_owned(), BTreeMap::new());
        let diagnostics: serde_json::Value =
            serde_json::from_str(&build.diagnostics).expect("JSON diagnostics");
        assert_eq!(diagnostics[0]["code"], "preview-build");
        assert_eq!(diagnostics[0]["message"], "missing include");
    }

    #[test]
    fn build_failures_retain_dependencies_from_the_last_usable_build() {
        let old = Dependency::contents(PathBuf::from("document.adoc"));
        let new = Dependency::contents(PathBuf::from(".adocweave.toml"));
        let old_fingerprint = Fingerprint::from_loaded_bytes(b"document");
        let new_fingerprint = Fingerprint::from_loaded_bytes(b"config");
        let previous = BTreeMap::from([(old.clone(), old_fingerprint.clone())]);
        let mut failure = Build::failure(
            "resource limit exceeded".to_owned(),
            BTreeMap::from([(new.clone(), new_fingerprint.clone())]),
        );

        failure.retain_dependencies_from(&previous);

        assert_eq!(failure.dependencies.get(&old), Some(&old_fingerprint));
        assert_eq!(failure.dependencies.get(&new), Some(&new_fingerprint));
    }

    #[test]
    fn typed_analysis_diagnostics_preserve_the_core_json_contract() {
        use adocweave_core::output::diagnostics::{
            Applicability, DiagnosticCode, DiagnosticId, Fix, RelatedInformation, Severity,
            TextEdit,
        };
        use adocweave_core::text::{TextRange, TextSize};

        let range = |start, end| {
            TextRange::new(
                TextSize::new(start).expect("small offset"),
                TextSize::new(end).expect("small offset"),
            )
            .expect("ordered range")
        };
        let diagnostics = vec![
            Diagnostic {
                id: DiagnosticId::new("later"),
                code: DiagnosticCode::new("z-code"),
                severity: Severity::Warning,
                message: "later message".to_owned(),
                range: range(8, 9),
                related: Vec::new(),
                fixes: Vec::new(),
            },
            Diagnostic {
                id: DiagnosticId::new("first"),
                code: DiagnosticCode::new("a-code"),
                severity: Severity::Error,
                message: "first message".to_owned(),
                range: range(1, 3),
                related: vec![RelatedInformation {
                    message: "related".to_owned(),
                    range: range(4, 6),
                }],
                fixes: vec![
                    Fix::new(
                        "replace",
                        Applicability::Maybe,
                        vec![TextEdit {
                            range: range(1, 2),
                            replacement: "x".to_owned(),
                        }],
                    )
                    .expect("valid fix"),
                ],
            },
        ];

        let expected: serde_json::Value =
            serde_json::from_str(&crate::diagnostic_output::render_json(&diagnostics))
                .expect("core diagnostics JSON");
        let actual: serde_json::Value = serde_json::from_str(&serialize_diagnostics(
            &PreviewDiagnostic::analysis(&diagnostics),
        ))
        .expect("preview diagnostics JSON");

        assert_eq!(actual, expected);
    }

    #[test]
    fn dependency_poll_schedule_is_independent_from_accept_retries() {
        let start = Instant::now();
        let mut poll = DependencyPoll::new(start);
        assert_eq!(poll.mode(start), Some(false));
        assert_eq!(poll.mode(start + ACCEPT_RETRY_INTERVAL), None);
        assert_eq!(poll.mode(start + DEPENDENCY_POLL_INTERVAL), Some(false));
        assert_eq!(
            poll.mode(start + DEPENDENCY_FORCE_HASH_INTERVAL),
            Some(true)
        );
    }

    #[test]
    fn changes_observed_during_the_delay_share_one_rebuild_deadline() {
        let start = Instant::now();
        let delay = Duration::from_millis(50);
        let mut debounce = ChangeDebounce::new(delay);

        debounce.observe(start);
        debounce.observe(start + Duration::from_millis(30));

        assert!(!debounce.ready(start + delay - Duration::from_millis(1)));
        assert!(debounce.ready(start + delay));

        debounce.clear();
        assert!(!debounce.ready(start + delay));

        debounce.restart(start + delay);
        assert!(!debounce.ready(start + delay));
        assert!(debounce.ready(start + delay + delay));
    }

    #[test]
    fn shutdown_cancels_the_initial_build_at_its_next_cooperative_boundary() {
        let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve port");
        let address = reservation.local_addr().expect("address");
        drop(reservation);
        let shutdown = Arc::new(AtomicBool::new(false));
        let server_shutdown = Arc::clone(&shutdown);
        let (started_sender, started_receiver) = std::sync::mpsc::channel();
        let (cancelled_sender, cancelled_receiver) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            run(
                Options {
                    bind: address.ip(),
                    port: address.port(),
                    debounce: Duration::from_millis(20),
                },
                |cancellation| {
                    started_sender.send(()).expect("started");
                    while !cancellation.is_cancelled() {
                        std::thread::yield_now();
                    }
                    cancelled_sender.send(()).expect("cancelled");
                    Err("cancelled".to_owned())
                },
                snapshot_dependencies,
                &server_shutdown,
            )
        });

        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("initial build started");
        shutdown.store(true, Ordering::Release);
        cancelled_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("initial build observed cancellation");
        server
            .join()
            .expect("server thread")
            .expect("clean shutdown");
    }

    #[test]
    fn fixed_routes_reload_by_generation_and_shutdown_releases_port() {
        let directory = tempfile::tempdir().expect("tempdir");
        let dependency = directory.path().join("dependency.adoc");
        let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve port");
        let address = reservation.local_addr().expect("address");
        drop(reservation);
        let shutdown = Arc::new(AtomicBool::new(false));
        let builds = Arc::new(AtomicU64::new(0));
        let server_shutdown = Arc::clone(&shutdown);
        let server_builds = Arc::clone(&builds);
        let server_dependency = dependency.clone();
        let thread = std::thread::spawn(move || {
            run(
                Options {
                    bind: address.ip(),
                    port: address.port(),
                    debounce: Duration::from_millis(20),
                },
                |_| {
                    let generation = server_builds.fetch_add(1, Ordering::Relaxed) + 1;
                    Ok(Build::new(
                        format!("<p>{generation}</p>"),
                        "[]".to_owned(),
                        snapshots([server_dependency.clone()]),
                    ))
                },
                snapshot_dependencies,
                &server_shutdown,
            )
        });

        let shell = request(address, "/");
        assert!(shell.starts_with("HTTP/1.1 200"), "{shell:?}");
        assert!(shell.contains("Content-Security-Policy: default-src 'none'"));
        assert!(shell.contains("<iframe"));
        assert!(request(address, "/secret").starts_with("HTTP/1.1 404"));
        let method_not_allowed = raw_request(
            address,
            &format!("POST / HTTP/1.1\r\nHost: {address}\r\n\r\n"),
        );
        assert!(method_not_allowed.starts_with("HTTP/1.1 405"));
        assert!(method_not_allowed.contains("\r\nAllow: GET, HEAD\r\n"));
        fs::write(&dependency, "created").expect("create dependency");
        for _ in 0..100 {
            if request(address, "/events").contains("\"generation\":2") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(builds.load(Ordering::Relaxed), 2);

        shutdown.store(true, Ordering::Release);
        thread.join().expect("server thread").expect("server");
        TcpListener::bind(address).expect("shutdown released port");
    }

    #[test]
    fn occupied_port_reports_the_bind_address() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("occupied port");
        let address = listener.local_addr().expect("address");
        let shutdown = AtomicBool::new(false);
        let error = run(
            Options {
                bind: address.ip(),
                port: address.port(),
                debounce: Duration::from_millis(20),
            },
            |_| unreachable!("binding fails before building"),
            snapshot_dependencies,
            &shutdown,
        )
        .expect_err("bind must fail");
        assert!(matches!(
            error,
            Error::Bind {
                address: failed,
                ..
            } if failed == address
        ));
    }

    #[test]
    fn cooperative_build_observes_cancellation_for_a_newer_change() {
        let directory = tempfile::tempdir().expect("tempdir");
        let dependency = directory.path().join("dependency.adoc");
        fs::write(&dependency, "one").expect("fixture");
        let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve port");
        let address = reservation.local_addr().expect("address");
        drop(reservation);
        let shutdown = Arc::new(AtomicBool::new(false));
        let builds = Arc::new(AtomicU64::new(0));
        let server_shutdown = Arc::clone(&shutdown);
        let server_builds = Arc::clone(&builds);
        let server_dependency = dependency.clone();
        let thread = std::thread::spawn(move || {
            run(
                Options {
                    bind: address.ip(),
                    port: address.port(),
                    debounce: Duration::from_millis(20),
                },
                |cancellation| {
                    let build = server_builds.fetch_add(1, Ordering::Relaxed) + 1;
                    if build == 2 {
                        while !cancellation.is_cancelled() {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        return Err("cancelled".to_owned());
                    }
                    Ok(Build::new(
                        format!("<p>{build}</p>"),
                        "[]".to_owned(),
                        snapshots([server_dependency.clone()]),
                    ))
                },
                snapshot_dependencies,
                &server_shutdown,
            )
        });
        request(address, "/events");
        fs::write(&dependency, "two").expect("first change");
        for _ in 0..100 {
            if builds.load(Ordering::Relaxed) >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        fs::write(&dependency, "six").expect("superseding change");
        for _ in 0..100 {
            if builds.load(Ordering::Relaxed) >= 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(builds.load(Ordering::Relaxed), 3);
        assert!(request(address, "/events").contains("\"generation\":2"));
        shutdown.store(true, Ordering::Release);
        thread.join().expect("server thread").expect("server");
    }

    #[test]
    fn dependency_changed_after_build_is_rejected_before_adoption() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = directory.path().join("root.adoc");
        let discovered = directory.path().join("new.adoc");
        fs::write(&root, "one").expect("root");
        fs::write(&discovered, "old").expect("dependency");
        let builds = AtomicU64::new(0);
        let mut first = Build::new(
            "initial".to_owned(),
            "[]".to_owned(),
            snapshots([root.clone()]),
        );
        fs::write(&root, "two").expect("trigger");
        let mut second = {
            builds.fetch_add(1, Ordering::Relaxed);
            let build = Build::new(
                "stale".to_owned(),
                "[]".to_owned(),
                snapshots([root, discovered.clone()]),
            );
            fs::write(&discovered, "new").expect("post-build change");
            build
        };
        assert!(first.changed(&mut snapshot_dependencies));
        assert!(second.changed(&mut snapshot_dependencies));
        assert_eq!(builds.load(Ordering::Relaxed), 1);
    }
}
