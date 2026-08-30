//! Typed `async-lsp` adapter with generation-checked background analysis.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::sync::Arc;

use adocweave::{CancellationCheck, CancellationToken};
use adocweave_project::process as process_project;
use async_lsp::client_monitor::ClientProcessMonitorLayer;
use async_lsp::concurrency::ConcurrencyLayer;
use async_lsp::lsp_types::{PublishDiagnosticsParams, Url, notification, request};
use async_lsp::panic::CatchUnwindLayer;
use async_lsp::router::Router;
use async_lsp::tracing::TracingLayer;
use async_lsp::{ClientSocket, ErrorCode, ResponseError};
use serde_json::Value;
use tokio::sync::Semaphore;
use tower::ServiceBuilder;

use crate::cancellation::{QueryCancellation, QueryError, QueryResult};
use crate::lifecycle::ProtocolLifecycleLayer;
use crate::service::{
    ProjectAnalysisAction, ProjectAnalysisCompletion, ProjectAnalysisOutcome, Session,
    project_observations_are_current,
};
use crate::state::{ProjectAnalysisSnapshot, ProjectSourceIndex};
use crate::workspace_scan::{
    WorkspaceScanRecovery, WorkspaceScanRecoveryTimer, WorkspaceScanStart, WorkspaceScanned,
};
use crate::{HostReferenceIndex, NoHostReferenceIndex};

const MAX_CONCURRENT_REQUESTS: usize = 16;
const MAX_CONCURRENT_ANALYSES: usize = 2;
const WATCH_SCAN_RECOVERY_DEBOUNCE_MS: u64 = 100;

pub(crate) struct Backend {
    client: ClientSocket,
    session: Session,
    cpu_limit: Arc<Semaphore>,
    analysis_workers: BTreeMap<String, AnalysisWorker>,
    workspace_scan_recovery_timer: WorkspaceScanRecoveryTimer,
}

struct AnalysisWorker {
    cancellation: Arc<CancellationToken>,
    handle: tokio::task::JoinHandle<()>,
}

struct ProjectProcessingCompleted(ProjectAnalysisCompletion);

struct ProjectValidationCompleted(ProjectAnalysisCompletion);

impl Backend {
    pub(crate) fn router(
        client: ClientSocket,
    ) -> impl async_lsp::LspService<Response = Value, Error = ResponseError> {
        Self::router_with_index(client, Arc::new(NoHostReferenceIndex))
    }

    pub(crate) fn router_with_index(
        client: ClientSocket,
        host_index: Arc<dyn HostReferenceIndex>,
    ) -> impl async_lsp::LspService<Response = Value, Error = ResponseError> {
        let process_monitor = client.clone();
        let mut router = Router::new(Self {
            client,
            session: Session::with_host_index(host_index),
            cpu_limit: Arc::new(Semaphore::new(MAX_CONCURRENT_ANALYSES)),
            analysis_workers: BTreeMap::new(),
            workspace_scan_recovery_timer: WorkspaceScanRecoveryTimer::default(),
        });

        router
            .request::<request::Initialize, _>(|state, params| {
                let response = state.session.initialize(&params);
                async move { Ok(response) }
            })
            .notification::<notification::Initialized>(|state, _| {
                state.register_dynamic_capabilities();
                ControlFlow::Continue(())
            })
            .request::<request::Shutdown, _>(|state, _| {
                state.cancel_all_analysis();
                state.invalidate_workspace_scan();
                async move { Ok(()) }
            })
            .notification::<notification::Exit>(|state, _| {
                state.cancel_all_analysis();
                state.invalidate_workspace_scan();
                ControlFlow::Continue(())
            })
            .notification::<notification::DidOpenTextDocument>(|state, params| {
                for job in state.session.begin_open(params) {
                    state.schedule_analysis(job);
                }
                ControlFlow::Continue(())
            })
            .notification::<notification::DidChangeTextDocument>(|state, params| {
                match state.session.begin_change(params) {
                    Ok(jobs) => {
                        for job in jobs {
                            state.schedule_analysis(job);
                        }
                        ControlFlow::Continue(())
                    }
                    Err(error) => ControlFlow::Break(Err(async_lsp::Error::Routing(error))),
                }
            })
            .notification::<notification::DidSaveTextDocument>(|state, params| {
                state.publish_current_diagnostics(params.text_document.uri)
            })
            .notification::<notification::DidChangeConfiguration>(|state, params| {
                if let Ok(jobs) = state.session.update_configuration(params.settings) {
                    for job in jobs {
                        state.schedule_analysis(job);
                    }
                }
                ControlFlow::Continue(())
            })
            .notification::<notification::DidChangeWatchedFiles>(|state, params| {
                let outcome = state.session.handle_workspace_files_changed(params);
                if outcome.cancel_recovery_timer {
                    state.cancel_workspace_scan_recovery();
                }
                if let Some(generation) = outcome.recovery_generation {
                    state.schedule_workspace_scan_recovery(generation);
                }
                for job in outcome.jobs {
                    state.schedule_analysis(job);
                }
                if let Some(start) = outcome.rebuild {
                    state.spawn_workspace_scan(start);
                }
                ControlFlow::Continue(())
            })
            .notification::<notification::DidChangeWorkspaceFolders>(|state, params| {
                for job in state.session.workspace_folders_changed(params) {
                    state.schedule_analysis(job);
                }
                ControlFlow::Continue(())
            })
            .notification::<notification::DidCloseTextDocument>(|state, params| {
                let uri = params.text_document.uri;
                state.cancel_analysis(uri.as_str());
                let outcome = state.session.close(&uri);
                if !outcome.closed {
                    return ControlFlow::Continue(());
                }
                for job in outcome.reanalysis_jobs {
                    state.schedule_analysis(job);
                }
                for diagnostic_uri in outcome.diagnostic_uris {
                    let Ok(diagnostic_uri) = diagnostic_uri.parse() else {
                        return ControlFlow::Break(Err(async_lsp::Error::Routing(format!(
                            "invalid diagnostic URI: {diagnostic_uri}"
                        ))));
                    };
                    if let ControlFlow::Break(error) =
                        state.publish_current_diagnostics(diagnostic_uri)
                    {
                        return ControlFlow::Break(error);
                    }
                }
                ControlFlow::Continue(())
            })
            .request::<request::DocumentSymbolRequest, _>(|state, params| {
                state.cpu_request(
                    params.text_document.uri,
                    move |service, uri, cancellation| {
                        service.document_symbols_cancellable(uri, cancellation)
                    },
                )
            })
            .request::<request::CodeActionRequest, _>(|state, params| {
                let range = params.range;
                let context = params.context;
                state.cpu_request(
                    params.text_document.uri,
                    move |service, uri, cancellation| {
                        service.code_actions_cancellable(uri, range, &context, cancellation)
                    },
                )
            })
            .request::<request::Formatting, _>(|state, params| {
                state.cpu_request(
                    params.text_document.uri,
                    move |service, uri, cancellation| {
                        service.formatting_cancellable(uri, cancellation)
                    },
                )
            })
            .request::<request::HoverRequest, _>(|state, params| {
                let request = params.text_document_position_params;
                let position = request.position;
                state.cpu_request(
                    request.text_document.uri,
                    move |service, uri, cancellation| {
                        service.hover_cancellable(uri, position, cancellation)
                    },
                )
            })
            .request::<request::Completion, _>(|state, params| {
                let request = params.text_document_position;
                let position = request.position;
                state.cpu_request(
                    request.text_document.uri,
                    move |service, uri, cancellation| {
                        service.completion_cancellable(uri, position, cancellation)
                    },
                )
            })
            .request::<request::GotoDefinition, _>(|state, params| {
                let request = params.text_document_position_params;
                let position = request.position;
                state.cpu_request(
                    request.text_document.uri,
                    move |service, uri, cancellation| {
                        service.definition_cancellable(uri, position, cancellation)
                    },
                )
            })
            .request::<request::References, _>(|state, params| {
                let request = params.text_document_position;
                let position = request.position;
                let include_declaration = params.context.include_declaration;
                state.cpu_request(
                    request.text_document.uri,
                    move |service, uri, cancellation| {
                        service.references_cancellable(
                            uri,
                            position,
                            include_declaration,
                            cancellation,
                        )
                    },
                )
            })
            .request::<request::DocumentLinkRequest, _>(|state, params| {
                state.cpu_request(
                    params.text_document.uri,
                    move |service, uri, cancellation| {
                        service.document_links_cancellable(uri, cancellation)
                    },
                )
            })
            .request::<request::SemanticTokensFullRequest, _>(|state, params| {
                state.cpu_request(
                    params.text_document.uri,
                    move |service, uri, cancellation| {
                        service.semantic_tokens_cancellable(uri, cancellation)
                    },
                )
            })
            .request::<request::PrepareRenameRequest, _>(|state, params| {
                let position = params.position;
                state.cpu_request(
                    params.text_document.uri,
                    move |service, uri, cancellation| {
                        service.prepare_rename_cancellable(uri, position, cancellation)
                    },
                )
            })
            .request::<request::Rename, _>(|state, params| {
                let request = params.text_document_position;
                let position = request.position;
                let new_name = params.new_name;
                state.cpu_request(
                    request.text_document.uri,
                    move |service, uri, cancellation| {
                        service.rename_cancellable(uri, position, &new_name, cancellation)
                    },
                )
            })
            .event::<ProjectProcessingCompleted>(|state, completed| {
                state.project_processing_completed(completed)
            })
            .event::<ProjectValidationCompleted>(|state, completed| {
                state.project_validation_completed(completed)
            });

        ServiceBuilder::new()
            .layer(TracingLayer::default())
            .layer(ProtocolLifecycleLayer)
            .layer(CatchUnwindLayer::default())
            .layer(ConcurrencyLayer::new(
                NonZeroUsize::new(MAX_CONCURRENT_REQUESTS).expect("non-zero request limit"),
            ))
            .layer(ClientProcessMonitorLayer::new(process_monitor))
            .service(router)
    }

    fn register_dynamic_capabilities(&mut self) {
        let Some(params) = self.session.watched_files_registration() else {
            return;
        };
        let client = self.client.clone();
        tokio::spawn(async move {
            let _ = client.request::<request::RegisterCapability>(params).await;
        });
    }

    /// Runs a read-only language request on the CPU pool with the shared
    /// cancellation and concurrency policy, resolving the document cancellation
    /// token before the request is scheduled.
    fn cpu_request<T, F>(
        &self,
        uri: Url,
        operation: F,
    ) -> impl std::future::Future<Output = Result<T, ResponseError>> + Send + use<T, F>
    where
        T: Send + 'static,
        F: FnOnce(&Session, &Url, &QueryCancellation) -> QueryResult<T> + Send + 'static,
    {
        let cancellation = self.session.document_cancellation(&uri);
        let service = self.session.clone();
        let limit = self.cpu_limit.clone();
        async move {
            run_cpu_request(limit, cancellation, move |cancellation| {
                operation(&service, &uri, cancellation)
            })
            .await
        }
    }

    fn spawn_workspace_scan(&self, start: WorkspaceScanStart) {
        let (sequence, cancellation) = start.into_parts();
        let service = self.session.clone();
        let client = self.client.clone();
        tokio::spawn(async move {
            let worker_cancellation = Arc::clone(&cancellation);
            let scan = tokio::task::spawn_blocking(move || {
                worker_cancellation.filesystem_job().map(|job| {
                    service.plan_workspace_scan_with_job(worker_cancellation.as_ref(), job)
                })
            })
            .await
            .map_err(|error| format!("workspace scan worker failed: {error}"))
            .and_then(|scan| scan);
            let _ = client.emit(WorkspaceScanned::new(sequence, scan));
        });
    }

    fn schedule_workspace_scan_recovery(&mut self, generation: u64) {
        self.cancel_workspace_scan_recovery();
        let client = self.client.clone();
        self.workspace_scan_recovery_timer
            .replace(tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(
                    WATCH_SCAN_RECOVERY_DEBOUNCE_MS,
                ))
                .await;
                let _ = client.emit(WorkspaceScanRecovery::new(generation));
            }));
    }

    fn cancel_workspace_scan_recovery(&mut self) {
        self.workspace_scan_recovery_timer.cancel();
    }

    fn invalidate_workspace_scan(&mut self) {
        self.cancel_workspace_scan_recovery();
        self.session.cancel_workspace_scan();
    }

    fn schedule_analysis(&mut self, snapshot: ProjectAnalysisSnapshot) {
        self.schedule_analysis_with_delay(snapshot, self.session.debounce_ms());
    }

    fn schedule_analysis_immediately(&mut self, snapshot: ProjectAnalysisSnapshot) {
        self.schedule_analysis_with_delay(snapshot, 0);
    }

    fn schedule_analysis_with_delay(
        &mut self,
        snapshot: ProjectAnalysisSnapshot,
        debounce_ms: u64,
    ) {
        self.cancel_analysis(&snapshot.uri);
        let limit = self.cpu_limit.clone();
        let client = self.client.clone();
        let uri = snapshot.uri.clone();
        let cancellation = Arc::clone(&snapshot.cancellation);
        let handle = tokio::spawn(async move {
            if debounce_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(debounce_ms)).await;
            }
            let Ok(_permit) = limit.acquire_owned().await else {
                return;
            };
            if snapshot.cancellation.is_cancelled() {
                return;
            }
            let result = tokio::task::spawn_blocking(move || {
                let mut snapshot = snapshot;
                let (outcome, source_index, observation_access) =
                    match snapshot.prepared_request.take() {
                        Some(project) => {
                            let source_index = project.source_index;
                            let observation_access = project.observation_access;
                            (
                                ProjectAnalysisOutcome::Processed(process_project(
                                    project.request,
                                    snapshot.cancellation.as_ref(),
                                )),
                                source_index,
                                Some(observation_access),
                            )
                        }
                        None => (
                            ProjectAnalysisOutcome::Rejected(
                                snapshot
                                    .project_problem
                                    .clone()
                                    .expect("a snapshot without a project request has a problem"),
                            ),
                            ProjectSourceIndex::default(),
                            None,
                        ),
                    };
                ProjectProcessingCompleted(ProjectAnalysisCompletion {
                    snapshot,
                    outcome,
                    source_index,
                    observation_access,
                    observations_are_current: None,
                })
            })
            .await;
            if let Ok(completed) = result {
                let _ = client.emit(completed);
            }
        });
        self.analysis_workers.insert(
            uri,
            AnalysisWorker {
                cancellation,
                handle,
            },
        );
    }

    fn project_processing_completed(
        &mut self,
        completed: ProjectProcessingCompleted,
    ) -> ControlFlow<async_lsp::Result<()>> {
        self.remove_completed_analysis_worker(&completed.0.snapshot);
        let next = self.session.project_processing_completed(completed.0);
        self.apply_analysis_next(next)
    }

    fn schedule_project_validation(&mut self, completion: ProjectAnalysisCompletion) {
        let limit = self.cpu_limit.clone();
        let client = self.client.clone();
        let uri = completion.snapshot.uri.clone();
        let cancellation = Arc::clone(&completion.snapshot.cancellation);
        let task_cancellation = Arc::clone(&cancellation);
        let handle = tokio::spawn(async move {
            let Ok(_permit) = limit.acquire_owned().await else {
                return;
            };
            if cancellation.is_cancelled() {
                return;
            }
            let result = tokio::task::spawn_blocking(move || {
                let mut completion = completion;
                completion.observations_are_current = Some(
                    completion
                        .observation_access
                        .as_ref()
                        .is_some_and(|access| {
                            let ProjectAnalysisOutcome::Processed(result) = &completion.outcome
                            else {
                                return true;
                            };
                            project_observations_are_current(result, access, cancellation.as_ref())
                        }),
                );
                ProjectValidationCompleted(completion)
            })
            .await;
            if let Ok(completed) = result {
                let _ = client.emit(completed);
            }
        });
        self.analysis_workers.insert(
            uri,
            AnalysisWorker {
                cancellation: task_cancellation,
                handle,
            },
        );
    }

    fn project_validation_completed(
        &mut self,
        completed: ProjectValidationCompleted,
    ) -> ControlFlow<async_lsp::Result<()>> {
        self.remove_completed_analysis_worker(&completed.0.snapshot);
        let next = self.session.complete_analysis(completed.0);
        self.apply_analysis_next(next)
    }

    fn remove_completed_analysis_worker(&mut self, snapshot: &ProjectAnalysisSnapshot) {
        if self
            .analysis_workers
            .get(&snapshot.uri)
            .is_some_and(|worker| Arc::ptr_eq(&worker.cancellation, &snapshot.cancellation))
        {
            self.analysis_workers.remove(&snapshot.uri);
        }
    }

    fn apply_analysis_next(
        &mut self,
        action: ProjectAnalysisAction,
    ) -> ControlFlow<async_lsp::Result<()>> {
        match action {
            ProjectAnalysisAction::Validate(completion) => {
                self.schedule_project_validation(*completion);
                ControlFlow::Continue(())
            }
            ProjectAnalysisAction::Retry(snapshot) => {
                self.schedule_analysis_immediately(snapshot);
                ControlFlow::Continue(())
            }
            ProjectAnalysisAction::Publish {
                snapshot,
                diagnostic_uris,
            } => self.publish_diagnostics_for_job(&snapshot, diagnostic_uris),
            ProjectAnalysisAction::Ignore => ControlFlow::Continue(()),
        }
    }

    fn publish_diagnostics_for_job(
        &mut self,
        job: &ProjectAnalysisSnapshot,
        additional_uris: impl IntoIterator<Item = String>,
    ) -> ControlFlow<async_lsp::Result<()>> {
        let mut publish_uris = job.previously_published_diagnostic_uris.clone();
        publish_uris.insert(job.uri.clone());
        publish_uris.extend(additional_uris);
        for uri in publish_uris {
            let Ok(uri) = uri.parse() else {
                return ControlFlow::Break(Err(async_lsp::Error::Routing(format!(
                    "invalid projected source URI: {uri}"
                ))));
            };
            if let ControlFlow::Break(error) = self.publish_current_diagnostics(uri) {
                return ControlFlow::Break(error);
            }
        }
        ControlFlow::Continue(())
    }

    fn cancel_analysis(&mut self, uri: &str) {
        if let Some(worker) = self.analysis_workers.remove(uri) {
            worker.handle.abort();
        }
    }

    fn cancel_all_analysis(&mut self) {
        self.session.shutdown();
        for (_, worker) in std::mem::take(&mut self.analysis_workers) {
            worker.handle.abort();
        }
    }

    fn publish_current_diagnostics(&mut self, uri: Url) -> ControlFlow<async_lsp::Result<()>> {
        let result = self
            .session
            .diagnostics(&uri)
            .map_err(async_lsp::Error::Routing)
            .and_then(|params: PublishDiagnosticsParams| {
                self.client
                    .notify::<notification::PublishDiagnostics>(params)?;
                Ok(())
            });
        match result {
            Ok(()) => ControlFlow::Continue(()),
            Err(error) => ControlFlow::Break(Err(error)),
        }
    }
}

struct CancelWorkerOnDrop(Arc<CancellationToken>);

impl Drop for CancelWorkerOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

async fn run_cpu_request<T, F>(
    limit: Arc<Semaphore>,
    document_cancellation: Option<Arc<CancellationToken>>,
    operation: F,
) -> Result<T, ResponseError>
where
    T: Send + 'static,
    F: FnOnce(&QueryCancellation) -> QueryResult<T> + Send + 'static,
{
    run_cpu_request_with_completion_hook(limit, document_cancellation, operation, || {}).await
}

async fn run_cpu_request_with_completion_hook<T, F, H>(
    limit: Arc<Semaphore>,
    document_cancellation: Option<Arc<CancellationToken>>,
    operation: F,
    after_worker: H,
) -> Result<T, ResponseError>
where
    T: Send + 'static,
    F: FnOnce(&QueryCancellation) -> QueryResult<T> + Send + 'static,
    H: FnOnce(),
{
    let request_cancellation = Arc::new(CancellationToken::new());
    let cancel_on_drop = CancelWorkerOnDrop(request_cancellation.clone());
    let permit = limit
        .acquire_owned()
        .await
        .map_err(|error| internal_error(error.to_string()))?;
    let cancellation = Arc::new(QueryCancellation::new(
        request_cancellation,
        document_cancellation,
    ));
    cancellation.check_now().map_err(query_response_error)?;
    let worker_cancellation = cancellation.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        (|| {
            worker_cancellation.check_now()?;
            let result = operation(&worker_cancellation);
            worker_cancellation.check_now()?;
            result
        })()
    })
    .await;
    after_worker();
    let response = finish_cpu_request(&cancellation, result);
    drop(cancel_on_drop);
    response
}

fn finish_cpu_request<T>(
    cancellation: &QueryCancellation,
    result: Result<QueryResult<T>, tokio::task::JoinError>,
) -> Result<T, ResponseError> {
    cancellation.check_now().map_err(query_response_error)?;
    let result =
        result.map_err(|error| internal_error(format!("request worker failed: {error}")))?;
    result.map_err(query_response_error)
}

fn query_response_error(error: QueryError) -> ResponseError {
    match error {
        QueryError::RequestCancelled => {
            ResponseError::new(ErrorCode::REQUEST_CANCELLED, "request was cancelled")
        }
        QueryError::ContentModified => content_modified(),
        QueryError::Internal(message) => internal_error(message),
    }
}

fn internal_error(error: impl ToString) -> ResponseError {
    ResponseError::new(ErrorCode::INTERNAL_ERROR, error.to_string())
}

fn content_modified() -> ResponseError {
    ResponseError::new(
        ErrorCode::CONTENT_MODIFIED,
        "document changed while the request was running",
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aborting_the_async_wrapper_does_not_release_a_running_workspace_worker() {
        let gate = Arc::new(Semaphore::new(1));
        let worker_gate = Arc::clone(&gate);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn(async move {
            let permit = worker_gate.acquire_owned().await.expect("workspace permit");
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                started_tx.send(()).expect("started receiver");
                finish_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("finish signal");
            })
            .await
            .expect("worker");
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker started");

        task.abort();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), Arc::clone(&gate).acquire_owned())
                .await
                .is_err(),
            "the detached blocking worker still owns the workspace gate"
        );
        finish_tx.send(()).expect("finish worker");
        let _permit = tokio::time::timeout(Duration::from_secs(1), gate.acquire_owned())
            .await
            .expect("workspace gate released after worker exit")
            .expect("workspace permit");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrency_cpu_requests_never_exceed_the_explicit_limit() {
        let limit = Arc::new(Semaphore::new(2));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let requests = (0..8).map(|_| {
            let active = active.clone();
            let maximum = maximum.clone();
            run_cpu_request(limit.clone(), None, move |_| {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(current, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(10));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
        });

        let results = futures::future::join_all(requests).await;
        assert!(results.into_iter().all(|result| result.is_ok()));
        assert_eq!(maximum.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancellation_dropping_a_request_cooperatively_cancels_its_worker() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn(run_cpu_request(
            Arc::new(Semaphore::new(1)),
            None,
            move |cancellation| {
                started_tx.send(()).expect("started receiver");
                while !cancellation.is_cancelled() {
                    std::thread::yield_now();
                }
                cancelled_tx.send(()).expect("cancelled receiver");
                Err::<(), _>(QueryError::RequestCancelled)
            },
        ));

        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker started");
        task.abort();
        cancelled_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker observed cancellation");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancellation_document_change_discards_a_completed_request() {
        let document_cancellation = Arc::new(CancellationToken::new());
        let worker_token = document_cancellation.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn(run_cpu_request(
            Arc::new(Semaphore::new(1)),
            Some(worker_token),
            move |_| {
                started_tx.send(()).expect("started receiver");
                finish_rx
                    .recv_timeout(Duration::from_secs(1))
                    .expect("finish signal");
                Ok(())
            },
        ));

        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker started");
        document_cancellation.cancel();
        finish_tx.send(()).expect("finish receiver");
        let error = task
            .await
            .expect("request task")
            .expect_err("content modified");

        assert_eq!(error.code, ErrorCode::CONTENT_MODIFIED);
    }

    #[test]
    fn document_change_after_worker_completion_overrides_success_and_internal_error() {
        fn assert_content_modified<T: std::fmt::Debug>(result: QueryResult<T>) {
            let document_cancellation = Arc::new(CancellationToken::new());
            let cancellation = QueryCancellation::new(
                Arc::new(CancellationToken::new()),
                Some(document_cancellation.clone()),
            );
            document_cancellation.cancel();

            let error =
                finish_cpu_request(&cancellation, Ok(result)).expect_err("content modified");
            assert_eq!(error.code, ErrorCode::CONTENT_MODIFIED);
        }

        assert_content_modified(Ok("completed result"));
        assert_content_modified(Err::<(), _>(QueryError::Internal(
            "query failed".to_owned(),
        )));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn asynchronous_path_checks_document_change_after_worker_completion() {
        async fn assert_content_modified(result: QueryResult<()>) {
            let document_cancellation = Arc::new(CancellationToken::new());
            let cancel_after_worker = document_cancellation.clone();
            let error = run_cpu_request_with_completion_hook(
                Arc::new(Semaphore::new(1)),
                Some(document_cancellation),
                move |_| result,
                move || cancel_after_worker.cancel(),
            )
            .await
            .expect_err("content modified");
            assert_eq!(error.code, ErrorCode::CONTENT_MODIFIED);
        }

        assert_content_modified(Ok(())).await;
        assert_content_modified(Err(QueryError::Internal("worker error".to_owned()))).await;
    }

    #[test]
    fn query_errors_have_distinct_protocol_codes() {
        assert_eq!(
            query_response_error(QueryError::RequestCancelled).code,
            ErrorCode::REQUEST_CANCELLED
        );
        assert_eq!(
            query_response_error(QueryError::ContentModified).code,
            ErrorCode::CONTENT_MODIFIED
        );
        assert_eq!(
            query_response_error(QueryError::Internal("broken query".to_owned())).code,
            ErrorCode::INTERNAL_ERROR
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancelled_workers_release_both_permits_for_the_next_request() {
        let limit = Arc::new(Semaphore::new(2));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let started_tx = started_tx.clone();
            let cancelled_tx = cancelled_tx.clone();
            tasks.push(tokio::spawn(run_cpu_request(
                limit.clone(),
                None,
                move |cancellation| {
                    started_tx.send(()).expect("started receiver");
                    while !cancellation.is_cancelled() {
                        std::thread::yield_now();
                    }
                    cancelled_tx.send(()).expect("cancelled receiver");
                    Err::<(), _>(QueryError::RequestCancelled)
                },
            )));
        }

        for _ in 0..2 {
            started_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("both workers started");
        }
        for task in &tasks {
            task.abort();
        }
        for _ in 0..2 {
            cancelled_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("both workers observed cancellation");
        }

        let result = tokio::time::timeout(
            Duration::from_secs(1),
            run_cpu_request(limit, None, |_| Ok("next request")),
        )
        .await
        .expect("third request acquired a released permit")
        .expect("third request succeeded");
        assert_eq!(result, "next request");
    }
}
