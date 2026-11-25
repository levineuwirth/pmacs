//! winit event-loop adapters for the shared daemon client.

use std::path::Path;

pub use pmacs_attach_client::*;
use pmacs_protocol::SessionToken;
use winit::event_loop::EventLoopProxy;

use crate::AppEvent;

pub fn connect(
    socket_path: &Path,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<AttachClient, AttachClientError> {
    connect_resuming(socket_path, None, proxy)
}

/// [`connect`], presenting `resume` in the session's v27 `Resume` step
/// (E9): the token a previous session of this window was given, so the
/// daemon starts the new socket on the view it retained for it. `None`
/// asks for a fresh view, which is what [`connect`] does.
pub fn connect_resuming(
    socket_path: &Path,
    resume: Option<SessionToken>,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<AttachClient, AttachClientError> {
    connect_with_sink_resuming(socket_path, resume, move |event| {
        proxy.send_event(AppEvent::Attach(event)).is_ok()
    })
}

/// Managed attach carrying an optional pre-window initial target.
pub fn connect_managed_with_target(
    socket_path: &Path,
    daemon_executable: &Path,
    initial_target: Option<InitialTargetPaths>,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<ManagedAttach, ManagedAttachError> {
    connect_managed_with_target_and_sink(
        socket_path,
        daemon_executable,
        initial_target,
        move |event| proxy.send_event(AppEvent::Attach(event)).is_ok(),
    )
}
