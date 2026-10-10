use super::*;

/// Run the headless server. This is the entry point called from main.rs.
pub fn run_server() -> io::Result<()> {
    crate::platform::ignore_server_hangup();
    let args: Vec<String> = std::env::args().collect();
    let handoff_import = args.get(2).map(String::as_str) == Some("--handoff-import");
    let process_context = crate::platform::prepare_server_process(handoff_import);
    init_logging();
    // Test-owned servers (HERDR_TEST_PARENT_PID) end with the test that spawned them.
    #[cfg(unix)]
    crate::server::test_parent_watchdog::spawn_from_env();
    match process_context {
        Ok(true) => info!("server using persistent user service context"),
        Ok(false) => {}
        Err(err) => {
            warn!(%err, "could not select persistent user service context; retaining inherited context")
        }
    }
    crate::platform::raise_server_nofile_limit();

    if handoff_import {
        let socket_path = args
            .get(3)
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing handoff socket"))?;
        let token = args
            .get(4)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing handoff token"))?;
        return run_handoff_import_server(&socket_path, token);
    }

    let loaded_config = config::Config::load();
    #[cfg(windows)]
    if loaded_config.config.server.allow_unelevated_clients {
        crate::platform::allow_unelevated_clients();
    }
    let (api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
    let event_hub = api::EventHub::default();
    let server_stop = crate::server::shutdown::ServerStop::default();

    // Start the JSON API socket server.
    let _api_server = match api::start_server_with_stop_control(
        api_tx.clone(),
        event_hub.clone(),
        server_stop.clone(),
    ) {
        Ok(server) => server,
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            eprintln!("error: herdr server is already running");
            eprintln!("api socket: {}", api::socket_path().display());
            std::process::exit(1);
        }
        Err(err) => return Err(err),
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;

    let result = run_then_shut_down(rt, |rt| {
        rt.block_on(async {
            // Create the App (with AppState, event channels, etc.).
            let mut app = app::App::try_new(
                &loaded_config.config,
                app::AppPolicy::PRODUCTION,
                config::config_diagnostic_summary(&loaded_config.diagnostics),
                api_rx,
                event_hub,
            )?;
            seed_startup_workspace_if_empty(&mut app);

            // Create the headless server.
            let mut server = match HeadlessServer::new(
                app,
                &loaded_config.diagnostics,
                Some(api_tx.clone()),
                Some(_api_server),
                server_stop,
            ) {
                Ok(server) => server,
                Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
                    eprintln!("error: herdr server is already running");
                    eprintln!("client socket: {}", client_socket_path().display());
                    std::process::exit(1);
                }
                Err(err) => return Err(err),
            };

            info!(
                api_socket = %api::socket_path().display(),
                client_socket = %client_socket_path().display(),
                "herdr server started"
            );
            print_ready_message(&api::socket_path(), &client_socket_path());
            server.app.run_plugin_startup_hooks();

            server.run().await
        })
    });

    crate::logging::shutdown("server");
    result
}

/// How long a server that is ending waits for blocking tasks still running. A
/// pane's child wait (`wait4` on a live shell) never ends on its own.
const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_millis(100);

/// Runs the server on `rt`, then shuts `rt` down within
/// [`RUNTIME_SHUTDOWN_GRACE`], also when the server panics. Dropping a runtime
/// during an unwind waits on every blocking task without a bound, so a panic
/// used to leave the process stuck in a pane's `wait4`, where SIGTERM no
/// longer reached anything (fork issue 197). The panic still surfaces once the
/// runtime is down.
fn run_then_shut_down<T>(
    rt: tokio::runtime::Runtime,
    run: impl FnOnce(&tokio::runtime::Runtime) -> T,
) -> T {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&rt)));
    rt.shutdown_timeout(RUNTIME_SHUTDOWN_GRACE);
    match outcome {
        Ok(value) => value,
        Err(panic) => {
            // A daemon's stderr goes nowhere; the log is the only trace.
            let message = panic
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            tracing::error!(%message, "server panicked; runtime shut down");
            crate::logging::shutdown("server");
            std::panic::resume_unwind(panic)
        }
    }
}

fn seed_startup_workspace_if_empty(app: &mut app::App) {
    let Some(cwd) = take_startup_cwd() else {
        return;
    };

    if !app.state.workspaces.is_empty() {
        info!(
            cwd = %cwd.display(),
            "restored session already has workspaces; ignoring startup cwd"
        );
        return;
    }

    match app.create_workspace_with_options(cwd.clone(), true) {
        Ok(_) => {
            info!(cwd = %cwd.display(), "created startup workspace");
        }
        Err(err) => {
            warn!(cwd = %cwd.display(), err = %err, "failed to create startup workspace");
            app.state.mode = app::Mode::Navigate;
        }
    }
}

fn take_startup_cwd() -> Option<PathBuf> {
    let cwd = std::env::var_os(crate::server::autodetect::STARTUP_CWD_ENV_VAR)?;
    std::env::remove_var(crate::server::autodetect::STARTUP_CWD_ENV_VAR);
    (!cwd.is_empty()).then(|| PathBuf::from(cwd))
}

#[cfg(unix)]
fn run_handoff_import_server(socket_path: &Path, token: &str) -> io::Result<()> {
    let loaded_config = config::Config::load();
    let mut received = crate::server::handoff::receive(socket_path, token)?;
    crate::server::handoff::log_import_result(received.manifest.panes.len());

    let (api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
    let event_hub = api::EventHub::default();
    let server_stop = crate::server::shutdown::ServerStop::default();

    let mut imports = HashMap::new();
    for (pane, fd) in received.manifest.panes.into_iter().zip(received.fds) {
        let pane_id = pane.pane_id;
        imports.insert(
            pane_id,
            crate::handoff_runtime::ImportedHandoffRuntime {
                master_fd: fd,
                state: pane,
            },
        );
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;

    let result = run_then_shut_down(rt, |rt| {
        rt.block_on(async {
            let app = app::App::new_from_handoff(
                &loaded_config.config,
                config::config_diagnostic_summary(&loaded_config.diagnostics),
                api_rx,
                event_hub.clone(),
                &received.manifest.snapshot,
                &mut imports,
            )?;
            crate::server::handoff::report_restored(&mut received.stream)?;
            if std::env::var("HERDR_TEST_HANDOFF_IMPORT_FAIL").as_deref() == Ok("after_restored") {
                return Err(io::Error::other(
                    "test handoff import failure after restored",
                ));
            }
            wait_for_old_public_sockets_to_close(Duration::from_secs(5))?;

            let api_server = api::start_server_with_stop_control(
                api_tx.clone(),
                event_hub.clone(),
                server_stop.clone(),
            )?;
            let mut server = HeadlessServer::new(
                app,
                &loaded_config.diagnostics,
                Some(api_tx.clone()),
                Some(api_server),
                server_stop,
            )?;
            // Carried across before any client attaches, so the first title sent is
            // the override rather than the configured one it replaced.
            server.api_window_title = received.manifest.api_window_title.take();
            // Keep panes at the size they had rather than shrinking them to the
            // headless default on the first frame, until a client reattaches.
            server.handoff_client_size = received.manifest.client_size;
            server.effective_size = server.detached_size();
            crate::server::handoff::report_ready(&mut received.stream)?;
            crate::server::handoff::wait_committed(&mut received.stream)?;
            server.app.assume_handoff_ownership();
            server.app.unpause_handoff_readers();
            server.begin_handoff_detection_sweep();
            if let Err(err) = crate::server::handoff::report_owned(&mut received.stream) {
                warn!(err = %err, "failed to report handoff ownership; continuing as owner");
            }
            info!("handoff import server started");
            print_ready_message(&api::socket_path(), &client_socket_path());
            server.app.run_plugin_startup_hooks();
            server.run().await
        })
    });

    crate::logging::shutdown("server");
    result
}

#[cfg(not(unix))]
fn run_handoff_import_server(_socket_path: &Path, _token: &str) -> io::Result<()> {
    Err(io::Error::other("live handoff is only supported on Unix"))
}

fn print_ready_message(api_socket: &Path, client_socket: &Path) {
    let message = format!(
        "herdr server running; you can use any herdr CLI command in another terminal.\n\
         api socket: {}\n\
         client socket: {}\n\
         logs: {}\n\
         did you mean to open the Herdr TUI? run `herdr`; you do not need `herdr server`.\n",
        api_socket.display(),
        client_socket.display(),
        crate::session::data_dir()
            .join("herdr-server.log")
            .display()
    );
    // The launching terminal may already be gone; the server keeps running.
    let _ = io::Write::write_all(&mut io::stderr(), message.as_bytes());
}

/// Initialize logging for the server process.
fn init_logging() {
    crate::logging::init_file_logging("herdr-server.log");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::AssertUnwindSafe;

    /// A runtime with a blocking task that never returns on its own, as a
    /// pane's child wait on a live shell. Runs `body` (which receives that
    /// runtime) through [`run_then_shut_down`] on its own thread and reports
    /// whether it returned (Ok(value)) or panicked (Err) within 10 s; `None`
    /// means it hung. The blocking task's channel stays open until then.
    fn with_endless_blocking_task<T: Send + 'static>(
        body: impl FnOnce() -> T + Send + 'static,
    ) -> Option<Result<T, ()>> {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .unwrap();
            let (keep_open, never) = std::sync::mpsc::channel::<()>();
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
                run_then_shut_down(rt, |rt| {
                    rt.block_on(async {
                        let (started_tx, started) = tokio::sync::oneshot::channel();
                        tokio::task::spawn_blocking(move || {
                            let _ = started_tx.send(());
                            let _ = never.recv();
                        });
                        let _ = started.await;
                    });
                    body()
                })
            }));
            drop(keep_open);
            let _ = done_tx.send(outcome.map_err(|_| ()));
        });
        done_rx.recv_timeout(Duration::from_secs(10)).ok()
    }

    #[test]
    fn a_server_that_ends_does_not_wait_on_an_endless_blocking_task() {
        assert_eq!(with_endless_blocking_task(|| 7), Some(Ok(7)));
    }

    /// Fork issue 197: a server that panicked dropped its runtime during the
    /// unwind, which waits on every blocking task without a bound, so the
    /// process hung in a pane's `wait4` where SIGTERM no longer reached it.
    #[test]
    fn a_server_that_panics_does_not_wait_on_an_endless_blocking_task() {
        assert_eq!(
            with_endless_blocking_task(|| -> u8 { panic!("the server failed") }),
            Some(Err(())),
            "the panic must surface instead of hanging in the runtime drop"
        );
    }
}
