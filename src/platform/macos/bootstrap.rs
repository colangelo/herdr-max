use std::ffi::{CStr, OsStr};
use std::io;
use std::process::Command;

use libc::{c_char, c_int, c_void, mach_port_t, uid_t};

const SERVER_CONTEXT_ENV: &str = "HERDR_MACOS_SERVER_CONTEXT";
const USER_CONTEXT: &str = "user";
const TASK_BOOTSTRAP_PORT: c_int = 4;
/// The libinfo service behind user, group, host and process-name lookups
/// (getpwuid, getaddrinfo, pgrep). It is the first thing a process loses with
/// its login session's bootstrap (fork issue 228).
const PROBE_SERVICE: &CStr = c"com.apple.system.opendirectoryd.libinfo";

type GetRoot = unsafe extern "C" fn(mach_port_t, *mut mach_port_t) -> c_int;
type LookupUser =
    unsafe extern "C" fn(mach_port_t, *const c_char, uid_t, *mut mach_port_t) -> c_int;
type LookUp = unsafe extern "C" fn(mach_port_t, *const c_char, *mut mach_port_t) -> c_int;
type SetSpecialPort = unsafe extern "C" fn(mach_port_t, c_int, mach_port_t) -> c_int;
type Deallocate = unsafe extern "C" fn(mach_port_t, mach_port_t) -> c_int;

unsafe extern "C" {
    static mut mach_task_self_: mach_port_t;
    fn task_set_special_port(task: mach_port_t, which: c_int, port: mach_port_t) -> c_int;
    fn mach_port_deallocate(task: mach_port_t, port: mach_port_t) -> c_int;
}

pub(crate) fn configure_server_daemon_context(command: &mut Command) {
    // Handoff inherits the source context, including intentionally direct launches.
    // Older replacements also cannot consume this marker and would leak it.
    if command.get_args().any(|arg| arg == "--handoff-import") {
        command.env_remove(SERVER_CONTEXT_ENV);
    } else {
        command.env(SERVER_CONTEXT_ENV, USER_CONTEXT);
    }
}

/// Called after exec, before logging, worker threads, or pane creation.
/// `handoff_user_context` is a live handoff whose caller asked the new server
/// to move to the per-user context (fork issue 226).
pub(crate) fn prepare_server_process(
    handoff_import: bool,
    handoff_user_context: bool,
) -> io::Result<bool> {
    let requested = std::env::var_os(SERVER_CONTEXT_ENV);
    // This is a one-shot launch request, not environment for panes or plugins.
    std::env::remove_var(SERVER_CONTEXT_ENV);
    if !needs_user_context(requested.as_deref(), handoff_import, handoff_user_context) {
        return Ok(false);
    }
    let api = BootstrapApi::load()?;
    api.adopt_user_context()?;
    Ok(true)
}

fn needs_user_context(
    requested: Option<&OsStr>,
    handoff_import: bool,
    handoff_user_context: bool,
) -> bool {
    // A replacement must preserve its source's policy, not silently rehome a
    // direct server. Fixed sources already pass on the durable context by fork.
    // Fork: a source started before that fix, or from a GUI terminal, keeps the
    // logout-scoped context forever unless the handoff's caller explicitly asks
    // to move it (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/226).
    if handoff_import {
        handoff_user_context
    } else {
        requested == Some(OsStr::new(USER_CONTEXT))
    }
}

/// What this process's Mach bootstrap reaches (fork issue 228). `None` when the
/// private bootstrap symbols are missing.
pub(crate) fn macos_services() -> Option<crate::platform::MacosServices> {
    BootstrapApi::load().ok().map(|api| api.services())
}

struct BootstrapApi {
    bootstrap: *mut mach_port_t,
    task: mach_port_t,
    get_root: GetRoot,
    lookup_user: LookupUser,
    look_up: LookUp,
    set_special_port: SetSpecialPort,
    deallocate: Deallocate,
}

impl BootstrapApi {
    fn load() -> io::Result<Self> {
        // These bootstrap interfaces are private macOS APIs, also used by tmux.
        // Resolve all symbols before changing anything so their removal cannot
        // prevent Herdr from launching with its inherited context.
        let bootstrap = symbol(c"bootstrap_port")?.cast::<mach_port_t>();
        let get_root = symbol(c"bootstrap_get_root")?;
        let lookup_user = symbol(c"bootstrap_look_up_per_user")?;
        let look_up = symbol(c"bootstrap_look_up")?;
        Ok(Self {
            bootstrap,
            task: unsafe { mach_task_self_ },
            get_root: unsafe { std::mem::transmute::<*mut c_void, GetRoot>(get_root) },
            lookup_user: unsafe { std::mem::transmute::<*mut c_void, LookupUser>(lookup_user) },
            look_up: unsafe { std::mem::transmute::<*mut c_void, LookUp>(look_up) },
            set_special_port: task_set_special_port,
            deallocate: mach_port_deallocate,
        })
    }

    fn adopt_user_context(&self) -> io::Result<()> {
        let inherited = unsafe { *self.bootstrap };
        let mut root = SendRight::new(self);
        check("bootstrap_get_root", unsafe {
            (self.get_root)(inherited, &mut root.port)
        })?;
        root.validate()?;

        let mut user = SendRight::new(self);
        check("bootstrap_look_up_per_user", unsafe {
            (self.lookup_user)(root.port, std::ptr::null(), libc::getuid(), &mut user.port)
        })?;
        user.validate()?;

        // setsid() does not detach from Aqua's Mach bootstrap namespace. Select
        // the per-user endpoint before any children inherit the logout-scoped one.
        check("task_set_bootstrap_port", unsafe {
            (self.set_special_port)(self.task, TASK_BOOTSTRAP_PORT, user.port)
        })?;
        unsafe {
            *self.bootstrap = user.port;
        }
        // The global now owns the lookup's send right, in addition to the task's
        // retained reference. Only release the old global after both are updated.
        user.port = 0;
        drop(SendRight {
            api: self,
            port: inherited,
        });
        Ok(())
    }
}

impl BootstrapApi {
    fn services(&self) -> crate::platform::MacosServices {
        let current = unsafe { *self.bootstrap };
        crate::platform::MacosServices {
            context: self.context_of(current),
            reachable: self.reaches(current, PROBE_SERVICE),
        }
    }

    /// The per-user bootstrap is one port: when this task already holds a send
    /// right to it, the lookup returns that same name.
    fn context_of(&self, current: mach_port_t) -> crate::platform::MacosServiceContext {
        use crate::platform::MacosServiceContext;
        let mut root = SendRight::new(self);
        if unsafe { (self.get_root)(current, &mut root.port) } != 0 || root.validate().is_err() {
            return MacosServiceContext::Unknown;
        }
        let mut user = SendRight::new(self);
        let status = unsafe {
            (self.lookup_user)(root.port, std::ptr::null(), libc::getuid(), &mut user.port)
        };
        if status != 0 || user.validate().is_err() {
            return MacosServiceContext::Unknown;
        }
        // Dropping `user` releases the reference this lookup added, even when
        // the name is the task's own bootstrap.
        if user.port == current {
            MacosServiceContext::User
        } else {
            MacosServiceContext::Session
        }
    }

    fn reaches(&self, current: mach_port_t, service: &CStr) -> bool {
        let mut port = SendRight::new(self);
        let status = unsafe { (self.look_up)(current, service.as_ptr(), &mut port.port) };
        status == 0 && port.validate().is_ok()
    }
}

fn symbol(name: &CStr) -> io::Result<*mut c_void> {
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if address.is_null() {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "macOS service-context API unavailable: {}",
                name.to_string_lossy()
            ),
        ))
    } else {
        Ok(address)
    }
}

fn check(operation: &str, status: c_int) -> io::Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{operation} failed (Mach error {status})"
        )))
    }
}

struct SendRight<'a> {
    api: &'a BootstrapApi,
    port: mach_port_t,
}

impl<'a> SendRight<'a> {
    fn new(api: &'a BootstrapApi) -> Self {
        Self { api, port: 0 }
    }

    fn validate(&self) -> io::Result<()> {
        if self.port == 0 || self.port == mach_port_t::MAX {
            Err(io::Error::other("macOS returned an invalid bootstrap port"))
        } else {
            Ok(())
        }
    }
}

impl Drop for SendRight<'_> {
    fn drop(&mut self) {
        if self.port != 0 && self.port != mach_port_t::MAX {
            unsafe {
                (self.api.deallocate)(self.api.task, self.port);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Calls {
        fail: Option<&'static str>,
        installed: Vec<mach_port_t>,
        released: Vec<mach_port_t>,
    }

    thread_local! {
        static CALLS: RefCell<Calls> = RefCell::default();
    }

    unsafe extern "C" fn get_root(_: mach_port_t, out: *mut mach_port_t) -> c_int {
        if CALLS.with(|calls| calls.borrow().fail == Some("root")) {
            return 5;
        }
        unsafe { *out = 20 };
        0
    }

    unsafe extern "C" fn lookup_user(
        _: mach_port_t,
        _: *const c_char,
        _: uid_t,
        out: *mut mach_port_t,
    ) -> c_int {
        let failure = CALLS.with(|calls| calls.borrow().fail);
        if failure == Some("lookup") {
            return 5;
        }
        unsafe { *out = if failure == Some("invalid") { 0 } else { 30 } };
        0
    }

    unsafe extern "C" fn look_up(
        current: mach_port_t,
        _: *const c_char,
        out: *mut mach_port_t,
    ) -> c_int {
        if CALLS.with(|calls| calls.borrow().fail == Some("look_up")) || current == DEAD {
            return 5;
        }
        unsafe { *out = 40 };
        0
    }

    /// A bootstrap whose login session is gone: every lookup through it fails.
    const DEAD: mach_port_t = 99;

    unsafe extern "C" fn set_special_port(_: mach_port_t, _: c_int, port: mach_port_t) -> c_int {
        CALLS.with(|calls| {
            let mut calls = calls.borrow_mut();
            if calls.fail == Some("install") {
                5
            } else {
                calls.installed.push(port);
                0
            }
        })
    }

    unsafe extern "C" fn deallocate(_: mach_port_t, port: mach_port_t) -> c_int {
        CALLS.with(|calls| calls.borrow_mut().released.push(port));
        0
    }

    fn fake_api(bootstrap: &mut mach_port_t) -> BootstrapApi {
        BootstrapApi {
            bootstrap,
            task: 1,
            get_root,
            lookup_user,
            look_up,
            set_special_port,
            deallocate,
        }
    }

    #[test]
    fn adoption_transfers_user_right_and_releases_old_and_root_rights() {
        CALLS.with(|calls| *calls.borrow_mut() = Calls::default());
        let mut bootstrap = 10;
        fake_api(&mut bootstrap).adopt_user_context().unwrap();
        assert_eq!(bootstrap, 30);
        CALLS.with(|calls| {
            let calls = calls.borrow();
            assert_eq!(calls.installed, [30]);
            assert_eq!(calls.released, [10, 20]);
        });
    }

    #[test]
    fn failed_adoption_keeps_inherited_context_and_releases_temporary_rights() {
        for (failure, released) in [
            ("root", vec![]),
            ("lookup", vec![20]),
            ("invalid", vec![20]),
            ("install", vec![30, 20]),
        ] {
            CALLS.with(|calls| {
                *calls.borrow_mut() = Calls {
                    fail: Some(failure),
                    ..Calls::default()
                }
            });
            let mut bootstrap = 10;
            assert!(fake_api(&mut bootstrap).adopt_user_context().is_err());
            assert_eq!(bootstrap, 10, "{failure}");
            CALLS.with(|calls| {
                let calls = calls.borrow();
                assert!(calls.installed.is_empty(), "{failure}");
                assert_eq!(calls.released, released, "{failure}");
            });
        }
    }

    /// The real APIs, not fakes: a child process whose bootstrap port is a dead
    /// name, as after a logout destroyed its login session, reports it lost. A
    /// child, so the test runner keeps its own bootstrap.
    #[test]
    fn a_dead_bootstrap_reads_as_unknown_and_unreachable() {
        unsafe extern "C" {
            fn mach_port_allocate(task: mach_port_t, right: c_int, name: *mut mach_port_t)
                -> c_int;
            fn mach_port_insert_right(
                task: mach_port_t,
                name: mach_port_t,
                poly: mach_port_t,
                poly_poly: c_int,
            ) -> c_int;
            fn mach_port_mod_refs(
                task: mach_port_t,
                name: mach_port_t,
                right: c_int,
                delta: c_int,
            ) -> c_int;
        }
        const MACH_PORT_RIGHT_RECEIVE: c_int = 1;
        const MACH_MSG_TYPE_MAKE_SEND: c_int = 20;

        let healthy = macos_services().expect("bootstrap symbols");
        assert!(healthy.reachable, "the test runner reaches libinfo");

        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            let report = unsafe {
                let task = mach_task_self_;
                let mut port = 0;
                let ok = mach_port_allocate(task, MACH_PORT_RIGHT_RECEIVE, &mut port) == 0
                    && mach_port_insert_right(task, port, port, MACH_MSG_TYPE_MAKE_SEND) == 0
                    // Destroying the receive right leaves a dead name behind.
                    && mach_port_mod_refs(task, port, MACH_PORT_RIGHT_RECEIVE, -1) == 0;
                let bootstrap = symbol(c"bootstrap_port")
                    .map(|address| address.cast::<mach_port_t>())
                    .ok();
                match (ok, bootstrap) {
                    (true, Some(bootstrap)) => {
                        *bootstrap = port;
                        match macos_services() {
                            Some(services) => {
                                u8::from(services.reachable)
                                    | (u8::from(
                                        services.context
                                            == crate::platform::MacosServiceContext::Unknown,
                                    ) << 1)
                            }
                            None => 0xfe,
                        }
                    }
                    _ => 0xff,
                }
            };
            unsafe {
                libc::write(fds[1], (&report as *const u8).cast(), 1);
                libc::_exit(0);
            }
        }
        unsafe { libc::close(fds[1]) };
        let mut report = 0u8;
        let read = unsafe { libc::read(fds[0], (&mut report as *mut u8).cast(), 1) };
        let mut status = 0;
        unsafe {
            libc::waitpid(pid, &mut status, 0);
            libc::close(fds[0]);
        }
        assert_eq!(read, 1, "the child reported");
        assert_ne!(report, 0xff, "the child could not make a dead bootstrap");
        assert_ne!(report, 0xfe, "the child lost the bootstrap symbols");
        assert_eq!(report & 1, 0, "a dead bootstrap must read as unreachable");
        assert_eq!(report & 2, 2, "a dead bootstrap has no readable context");
    }

    #[test]
    fn missing_private_symbol_is_a_recoverable_error() {
        let error = symbol(c"herdr_nonexistent_bootstrap_symbol").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn direct_and_handoff_servers_keep_inherited_context() {
        assert!(!needs_user_context(None, false, false));
        assert!(!needs_user_context(
            Some(OsStr::new("unknown")),
            false,
            false
        ));
        assert!(needs_user_context(
            Some(OsStr::new(USER_CONTEXT)),
            false,
            false
        ));
        assert!(!needs_user_context(None, true, false));
        assert!(!needs_user_context(
            Some(OsStr::new(USER_CONTEXT)),
            true,
            false
        ));
    }

    #[test]
    fn a_handoff_moves_to_the_user_context_only_when_its_caller_asks() {
        assert!(needs_user_context(None, true, true));
        assert!(needs_user_context(
            Some(OsStr::new(USER_CONTEXT)),
            true,
            true
        ));
        // The request belongs to the handoff; a direct launch ignores it.
        assert!(!needs_user_context(None, false, true));
    }

    #[test]
    fn services_report_the_user_context_when_the_task_holds_the_per_user_bootstrap() {
        CALLS.with(|calls| *calls.borrow_mut() = Calls::default());
        let mut bootstrap = 30;
        let services = fake_api(&mut bootstrap).services();
        assert_eq!(services.context, crate::platform::MacosServiceContext::User);
        assert!(services.reachable);
        CALLS.with(|calls| {
            let calls = calls.borrow();
            // Root, the per-user lookup's extra reference, then the probe port.
            assert_eq!(calls.released, [30, 20, 40]);
            assert!(calls.installed.is_empty());
        });
    }

    #[test]
    fn services_report_a_session_context_and_a_lost_one() {
        CALLS.with(|calls| *calls.borrow_mut() = Calls::default());
        let mut bootstrap = 10;
        let services = fake_api(&mut bootstrap).services();
        assert_eq!(
            services.context,
            crate::platform::MacosServiceContext::Session
        );
        assert!(services.reachable);

        let mut bootstrap = DEAD;
        CALLS.with(|calls| {
            *calls.borrow_mut() = Calls {
                fail: Some("root"),
                ..Calls::default()
            }
        });
        let services = fake_api(&mut bootstrap).services();
        assert_eq!(
            services.context,
            crate::platform::MacosServiceContext::Unknown
        );
        assert!(!services.reachable);
        assert_eq!(bootstrap, DEAD, "probing never changes the bootstrap");
    }

    #[test]
    fn handoff_commands_remove_even_an_inherited_launch_marker() {
        let mut command = Command::new("herdr");
        command.args(["server", "--handoff-import", "socket", "token"]);
        command.env(SERVER_CONTEXT_ENV, USER_CONTEXT);
        crate::platform::detach_server_daemon_command(&mut command);
        assert!(command
            .get_envs()
            .any(|(key, value)| key == SERVER_CONTEXT_ENV && value.is_none()));
    }

    #[test]
    fn only_detached_server_commands_request_user_context() {
        let direct = Command::new("herdr");
        assert!(!direct.get_envs().any(|(key, _)| key == SERVER_CONTEXT_ENV));
        let mut detached = Command::new("herdr");
        crate::platform::detach_server_daemon_command(&mut detached);
        assert!(detached.get_envs().any(|(key, value)| {
            key == SERVER_CONTEXT_ENV && value == Some(OsStr::new(USER_CONTEXT))
        }));
    }
}
