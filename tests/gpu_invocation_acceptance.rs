//! End-to-end acceptance for one-command GPU invocation and managed daemon lifecycle.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

const TEST_GPU_OVERRIDE: &str = "PMACS_TEST_GPU_BIN";

fn secure_tempdir() -> TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700))
        .expect("chmod tempdir 0700");
    temp
}

fn write_script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\nset -eu\n{body}\n")).expect("write script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod script");
}

#[cfg(not(feature = "crdt"))]
#[test]
fn non_crdt_root_rejects_gpu_before_socket_io_discovery_or_spawn() {
    let temp = secure_tempdir();
    let runtime = temp.path().join("runtime");
    fs::create_dir(&runtime).expect("create runtime");
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).expect("chmod runtime 0700");
    let fake_gpu = temp.path().join("fake-gpu");
    let marker = temp.path().join("spawned");
    write_script(&fake_gpu, "touch \"$PMACS_TEST_MARKER\"");

    let output = Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .arg("--gpu")
        .env(TEST_GPU_OVERRIDE, &fake_gpu)
        .env("PMACS_TEST_MARKER", &marker)
        .env("XDG_RUNTIME_DIR", &runtime)
        .output()
        .expect("run non-CRDT pmacs --gpu");
    assert!(!output.status.success());
    assert!(!marker.exists(), "GPU executable must not be spawned");
    assert!(
        !runtime.join("pmacs/default.sock").exists(),
        "the CRDT gate must run before default-socket creation"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--features crdt"),
        "unexpected stderr: {stderr}"
    );

    let occupied_socket = temp.path().join("occupied.sock");
    let listener =
        std::os::unix::net::UnixListener::bind(&occupied_socket).expect("bind occupied socket");
    let occupied = Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .args(["--gpu", "--socket"])
        .arg(&occupied_socket)
        .env(TEST_GPU_OVERRIDE, &fake_gpu)
        .env("PMACS_TEST_MARKER", &marker)
        .output()
        .expect("run non-CRDT pmacs --gpu against occupied socket");
    assert!(!occupied.status.success());
    assert!(
        !marker.exists(),
        "live socket must not weaken the CRDT gate"
    );
    assert!(
        occupied_socket.exists(),
        "live socket must remain untouched"
    );
    drop(listener);
}

#[cfg(feature = "crdt")]
mod crdt {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::os::unix::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Stdio;
    use std::process::{Child, ChildStdin};
    use std::thread;
    use std::time::{Duration, Instant};

    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    use pmacs::cell::CellSize;
    use pmacs::crdt::CrdtState;
    use pmacs::protocol::{
        AttachRequest, FrontendCapabilities, FrontendEvent, FrontendId, Hello, InitialTarget,
        InitialTargetResult, InstanceCapabilities, InstanceIdentity, InstanceMessage,
        PROTOCOL_VERSION, SessionBootstrapRequest,
    };
    use pmacs::transport::{read_message, write_message};

    use super::*;

    fn pmacs_binary() -> PathBuf {
        PathBuf::from(env!("CARGO_BIN_EXE_pmacs"))
    }

    fn gpu_binary() -> PathBuf {
        pmacs_binary()
            .parent()
            .expect("test binary directory")
            .join("pmacs-gpu")
    }

    fn parse_report(report: &Path) -> HashMap<String, String> {
        fs::read_to_string(report)
            .expect("read probe report")
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }

    fn wait_for_fact(
        report: &Path,
        key: &str,
        expected: &str,
        timeout: Duration,
    ) -> HashMap<String, String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if report.exists() {
                let facts = parse_report(report);
                if facts.get(key).is_some_and(|value| value == expected) {
                    return facts;
                }
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "report {} did not reach {key}={expected}: {}",
            report.display(),
            fs::read_to_string(report).unwrap_or_default()
        );
    }

    fn signal_pid(pid: u32, signal: Signal) {
        let _ = kill(Pid::from_raw(pid.cast_signed()), signal);
    }

    fn wait_for_exit(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = child.try_wait().expect("inspect child") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "child did not exit within {timeout:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_daemon(socket: &Path, child: &mut Child) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(mut stream) = UnixStream::connect(socket) {
                let _: Hello = read_message(&mut stream).expect("read daemon Hello");
                return;
            }
            if let Some(status) = child.try_wait().expect("inspect daemon") {
                panic!("daemon exited before listening: {status}");
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("daemon did not listen on {}", socket.display());
    }

    fn attach_surviving_frontend(socket: &Path) -> (FrontendId, UnixStream) {
        let mut stream = UnixStream::connect(socket).expect("connect surviving frontend");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set surviving frontend timeout");
        let hello: Hello = read_message(&mut stream).expect("surviving frontend Hello");
        write_message(
            &mut stream,
            &AttachRequest {
                protocol_version: PROTOCOL_VERSION,
                frontend_capabilities: FrontendCapabilities {
                    multi_frontend: true,
                    crdt_replica: true,
                    ..FrontendCapabilities::default()
                },
                initial_size: CellSize::new(24, 80),
            },
        )
        .expect("attach surviving frontend");

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_snapshot = false;
        let mut saw_full_grid = false;
        while !(saw_snapshot && saw_full_grid) {
            assert!(
                Instant::now() < deadline,
                "surviving frontend did not initialize"
            );
            match read_message::<InstanceMessage>(&mut stream).expect("initialize survivor") {
                InstanceMessage::BufferSnapshot { .. } => saw_snapshot = true,
                InstanceMessage::CellDelta {
                    full_grid: true, ..
                } => saw_full_grid = true,
                _ => {}
            }
        }
        (hello.assigned_frontend_id, stream)
    }

    struct TargetSession {
        frontend_id: FrontendId,
        buffer_id: pmacs::buffer::BufferId,
        replica: CrdtState,
        stream: UnixStream,
    }

    fn attach_target(socket: &Path, cwd: &Path, path: &Path) -> TargetSession {
        use std::os::unix::ffi::OsStrExt;

        let mut stream = UnixStream::connect(socket).expect("connect target frontend");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set target frontend timeout");
        let hello: Hello = read_message(&mut stream).expect("target frontend Hello");
        assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
        write_message(
            &mut stream,
            &AttachRequest {
                protocol_version: PROTOCOL_VERSION,
                frontend_capabilities: FrontendCapabilities {
                    multi_frontend: true,
                    crdt_replica: true,
                    semantic_render: true,
                    ..FrontendCapabilities::default()
                },
                initial_size: CellSize::new(24, 80),
            },
        )
        .expect("attach target frontend");
        write_message(
            &mut stream,
            &SessionBootstrapRequest {
                initial_target: Some(InitialTarget {
                    cwd: cwd.as_os_str().as_bytes().to_vec(),
                    path: path.as_os_str().as_bytes().to_vec(),
                }),
            },
        )
        .expect("send initial target");

        let (buffer_id, snapshot) =
            match read_message::<InstanceMessage>(&mut stream).expect("target snapshot") {
                InstanceMessage::BufferSnapshot {
                    buffer_id,
                    crdt_snapshot,
                } => (buffer_id, crdt_snapshot),
                other => panic!("expected target snapshot first, got {other:?}"),
            };
        assert_eq!(
            read_message::<InstanceMessage>(&mut stream).expect("target result"),
            InstanceMessage::InitialTargetResult(InitialTargetResult::Opened { buffer_id })
        );
        let replica = CrdtState::new(hello.assigned_frontend_id.0).expect("target replica");
        replica
            .import_snapshot(&snapshot)
            .expect("import target snapshot");
        TargetSession {
            frontend_id: hello.assigned_frontend_id,
            buffer_id,
            replica,
            stream,
        }
    }

    fn open_raw_target(
        socket: &Path,
        cwd: Vec<u8>,
        path: Vec<u8>,
    ) -> (FrontendId, UnixStream, Vec<InstanceMessage>) {
        let mut stream = UnixStream::connect(socket).expect("connect raw target frontend");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set raw target timeout");
        let hello: Hello = read_message(&mut stream).expect("raw target Hello");
        write_message(
            &mut stream,
            &AttachRequest {
                protocol_version: hello.protocol_version,
                frontend_capabilities: FrontendCapabilities {
                    multi_frontend: true,
                    crdt_replica: true,
                    semantic_render: true,
                    ..FrontendCapabilities::default()
                },
                initial_size: CellSize::new(24, 80),
            },
        )
        .expect("attach raw target frontend");
        write_message(
            &mut stream,
            &SessionBootstrapRequest {
                initial_target: Some(InitialTarget { cwd, path }),
            },
        )
        .expect("send raw target");

        let first = read_message::<InstanceMessage>(&mut stream).expect("raw target result");
        let messages = if matches!(first, InstanceMessage::BufferSnapshot { .. }) {
            vec![
                first,
                read_message::<InstanceMessage>(&mut stream).expect("raw opened result"),
            ]
        } else {
            vec![first]
        };
        (hello.assigned_frontend_id, stream, messages)
    }

    fn request_raw_target(socket: &Path, cwd: Vec<u8>, path: Vec<u8>) -> Vec<InstanceMessage> {
        open_raw_target(socket, cwd, path).2
    }

    fn spawn_daemon(socket: &Path, envs: &[(&str, &str)]) -> Child {
        let home = socket.parent().expect("socket parent");
        let mut command = Command::new(pmacs_binary());
        command
            .args(["--daemon", "--socket"])
            .arg(socket)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (key, value) in envs {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("spawn daemon");
        wait_for_daemon(socket, &mut child);
        child
    }

    struct ManagedProbe {
        child: Child,
        stdin: Option<ChildStdin>,
        report: PathBuf,
        daemon_pid: Option<u32>,
    }

    impl ManagedProbe {
        fn spawn(socket: &Path, report: &Path, daemon_executable: &Path, home: &Path) -> Self {
            Self::spawn_with_env(socket, report, daemon_executable, home, &[])
        }

        fn spawn_target(
            socket: &Path,
            report: &Path,
            daemon_executable: &Path,
            home: &Path,
            cwd: &Path,
            target: &Path,
        ) -> Self {
            Self::spawn_with_env_and_target(
                socket,
                report,
                daemon_executable,
                home,
                &[],
                Some((cwd, target)),
            )
        }

        fn spawn_with_env(
            socket: &Path,
            report: &Path,
            daemon_executable: &Path,
            home: &Path,
            envs: &[(&str, &Path)],
        ) -> Self {
            Self::spawn_with_env_and_target(socket, report, daemon_executable, home, envs, None)
        }

        fn spawn_with_env_and_target(
            socket: &Path,
            report: &Path,
            daemon_executable: &Path,
            home: &Path,
            envs: &[(&str, &Path)],
            initial_target: Option<(&Path, &Path)>,
        ) -> Self {
            assert!(
                gpu_binary().is_file(),
                "build pmacs-gpu before this acceptance suite"
            );
            let mut command = Command::new(gpu_binary());
            command
                .args(["--headless-managed-probe"])
                .arg(socket)
                .arg(report)
                .arg(daemon_executable);
            if let Some((cwd, path)) = initial_target {
                command.arg("--initial-target").arg(cwd).arg(path);
            }
            command
                .env("HOME", home)
                .env("XDG_CONFIG_HOME", home)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            for (key, value) in envs {
                command.env(key, value);
            }
            let mut child = command.spawn().expect("spawn managed probe");
            let stdin = child.stdin.take().expect("probe stdin");
            Self {
                child,
                stdin: Some(stdin),
                report: report.to_owned(),
                daemon_pid: None,
            }
        }

        fn wait_ready(&mut self) -> HashMap<String, String> {
            let facts = wait_for_fact(&self.report, "phase", "ready", Duration::from_secs(10));
            if facts
                .get("spawned_daemon")
                .is_some_and(|value| value == "true")
            {
                self.daemon_pid = facts.get("daemon_pid").and_then(|value| value.parse().ok());
            }
            facts
        }

        fn close(mut self) -> std::process::ExitStatus {
            self.stdin.take();
            wait_for_fact(&self.report, "phase", "complete", Duration::from_secs(5));
            wait_for_exit(&mut self.child, Duration::from_secs(5))
        }
    }

    impl Drop for ManagedProbe {
        fn drop(&mut self) {
            self.stdin.take();
            let _ = self.child.kill();
            let _ = self.child.wait();
            let daemon_reaped = fs::read_to_string(&self.report)
                .ok()
                .is_some_and(|report| report.lines().any(|line| line == "daemon_reaped=true"));
            if !daemon_reaped && let Some(pid) = self.daemon_pid {
                signal_pid(pid, Signal::SIGTERM);
            }
        }
    }

    #[test]
    fn root_broker_forwards_resolved_arguments_and_gpu_outcome() {
        let temp = secure_tempdir();
        let fake_gpu = temp.path().join("fake-gpu");
        let record = temp.path().join("argv");
        let socket = temp.path().join("broker.sock");
        let launch_cwd = temp.path().join("launch");
        let launcher_home = temp.path().join("home");
        fs::create_dir(&launch_cwd).expect("create launcher cwd");
        fs::create_dir(&launcher_home).expect("create launcher home");
        write_script(
            &fake_gpu,
            "printf '%s\\n' \"$@\" > \"$PMACS_TEST_RECORD\"\nexit \"$PMACS_TEST_EXIT\"",
        );

        let success = Command::new(pmacs_binary())
            .args(["--gpu", "--socket"])
            .arg(&socket)
            .arg("~/notes.txt")
            .current_dir(&launch_cwd)
            .env("HOME", &launcher_home)
            .env(TEST_GPU_OVERRIDE, &fake_gpu)
            .env("PMACS_TEST_RECORD", &record)
            .env("PMACS_TEST_EXIT", "0")
            .output()
            .expect("run root broker success");
        assert!(
            success.status.success(),
            "{}",
            String::from_utf8_lossy(&success.stderr)
        );
        let argv = fs::read_to_string(&record).expect("read forwarded argv");
        let args = argv.lines().collect::<Vec<_>>();
        assert_eq!(args[0], "--managed-attach");
        assert_eq!(Path::new(args[1]), socket);
        assert_eq!(Path::new(args[2]), pmacs_binary());
        assert_eq!(args[3], "--initial-target");
        assert_eq!(Path::new(args[4]), launch_cwd);
        assert_eq!(Path::new(args[5]), launcher_home.join("notes.txt"));

        let failure = Command::new(pmacs_binary())
            .arg("--gpu")
            .env(TEST_GPU_OVERRIDE, &fake_gpu)
            .env("PMACS_TEST_RECORD", &record)
            .env("PMACS_TEST_EXIT", "23")
            .output()
            .expect("run root broker failure");
        assert_eq!(failure.status.code(), Some(23));

        let missing = temp.path().join("missing-gpu");
        let spawn_failure = Command::new(pmacs_binary())
            .arg("--gpu")
            .env(TEST_GPU_OVERRIDE, &missing)
            .output()
            .expect("run root broker spawn failure");
        assert!(!spawn_failure.status.success());
        assert!(
            String::from_utf8_lossy(&spawn_failure.stderr).contains(&*missing.to_string_lossy())
        );
    }

    #[test]
    fn one_command_root_broker_reaches_target_ready_through_the_real_gpu_connector() {
        let temp = secure_tempdir();
        let cwd = temp.path().join("workspace");
        fs::create_dir(&cwd).expect("create workspace");
        fs::write(cwd.join("opened.txt"), "opened by root\n").expect("write target");
        let socket = temp.path().join("one-command.sock");
        let report = temp.path().join("one-command-report");
        let wrapper = temp.path().join("headless-gpu");
        write_script(
            &wrapper,
            "test \"$1\" = \"--managed-attach\"\n\
             socket=$2\n\
             daemon=$3\n\
             shift 3\n\
             exec \"$PMACS_REAL_GPU\" --headless-managed-probe \
             \"$socket\" \"$PMACS_TEST_REPORT\" \"$daemon\" \"$@\"",
        );

        let output = Command::new(pmacs_binary())
            .args(["--gpu", "--socket"])
            .arg(&socket)
            .arg("opened.txt")
            .current_dir(&cwd)
            .env(TEST_GPU_OVERRIDE, &wrapper)
            .env("PMACS_REAL_GPU", gpu_binary())
            .env("PMACS_TEST_REPORT", &report)
            .env("HOME", temp.path())
            .env("XDG_CONFIG_HOME", temp.path())
            .output()
            .expect("run one-command target flow");
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let facts = parse_report(&report);
        assert_eq!(facts.get("phase").map(String::as_str), Some("complete"));
        assert_eq!(
            facts
                .get("server_protocol_version")
                .and_then(|value| value.parse::<u32>().ok()),
            Some(PROTOCOL_VERSION)
        );
        assert_eq!(
            facts.get("spawned_daemon").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn target_bootstrap_is_snapshot_first_deduplicated_and_identical_for_daemon_reuse_or_spawn() {
        let temp = secure_tempdir();
        let cwd = temp.path().join("workspace");
        fs::create_dir(&cwd).expect("create workspace");
        fs::write(cwd.join("alpha.txt"), "alpha\n").expect("write alpha");
        fs::write(cwd.join("beta.txt"), "beta\n").expect("write beta");
        let raw_name = OsString::from_vec(vec![b'r', b'a', b'w', 0xff]);
        fs::write(cwd.join(&raw_name), "raw\n").expect("write non-UTF-8 target");

        let existing_socket = temp.path().join("existing-target.sock");
        let mut daemon = spawn_daemon(&existing_socket, &[]);
        let mut alpha = attach_target(&existing_socket, &cwd, Path::new("alpha.txt"));
        let mut same_alpha = attach_target(&existing_socket, &cwd, Path::new("./alpha.txt"));
        let beta = attach_target(&existing_socket, &cwd, Path::new("nested/../beta.txt"));
        assert_eq!(alpha.replica.materialize_string(), "alpha\n");
        assert_eq!(same_alpha.buffer_id, alpha.buffer_id);
        assert_eq!(beta.replica.materialize_string(), "beta\n");
        assert_ne!(beta.buffer_id, alpha.buffer_id);
        let raw = attach_target(&existing_socket, &cwd, Path::new(raw_name.as_os_str()));
        assert_eq!(raw.replica.materialize_string(), "raw\n");
        let missing_path = Path::new("new-draft.txt");
        let missing = attach_target(&existing_socket, &cwd, missing_path);
        assert_eq!(missing.replica.materialize_string(), "");
        assert!(!cwd.join(missing_path).exists());
        let same_missing = attach_target(&existing_socket, &cwd, Path::new("./new-draft.txt"));
        assert_eq!(same_missing.buffer_id, missing.buffer_id);

        let version = alpha.replica.version();
        let alpha_len = alpha.replica.len_utf8();
        alpha
            .replica
            .insert(alpha_len, "unsaved")
            .expect("optimistic alpha edit");
        let op_bytes = alpha
            .replica
            .export_updates_since(&version)
            .expect("export alpha edit");
        write_message(
            &mut alpha.stream,
            &FrontendEvent::CrdtOp {
                frontend_id: alpha.frontend_id,
                buffer_id: alpha.buffer_id,
                op: pmacs::rope::CrdtOp {
                    peer_id: alpha.frontend_id.0,
                    bytes: op_bytes,
                },
            },
        )
        .expect("send alpha edit");
        loop {
            match read_message::<InstanceMessage>(&mut same_alpha.stream)
                .expect("read alpha broadcast")
            {
                InstanceMessage::CrdtOp { buffer_id, op } if buffer_id == alpha.buffer_id => {
                    same_alpha
                        .replica
                        .import_updates(&op.bytes)
                        .expect("import alpha broadcast");
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(same_alpha.replica.materialize_string(), "alpha\nunsaved");

        let reopened = attach_target(&existing_socket, &cwd, &cwd.join("alpha.txt"));
        assert_eq!(reopened.buffer_id, alpha.buffer_id);
        assert_eq!(reopened.replica.materialize_string(), "alpha\nunsaved");

        signal_pid(daemon.id(), Signal::SIGTERM);
        assert!(wait_for_exit(&mut daemon, Duration::from_secs(5)).success());

        let spawned_socket = temp.path().join("spawned-target.sock");
        let report = temp.path().join("target-report");
        let mut probe = ManagedProbe::spawn_target(
            &spawned_socket,
            &report,
            &pmacs_binary(),
            temp.path(),
            &cwd,
            Path::new("beta.txt"),
        );
        let facts = probe.wait_ready();
        assert_eq!(
            facts.get("spawned_daemon").map(String::as_str),
            Some("true")
        );
        assert!(probe.close().success());
    }

    #[test]
    fn directory_target_reaches_ready_and_leaves_the_daemon_usable() {
        let temp = secure_tempdir();
        let socket = temp.path().join("directory-target.sock");
        let mut daemon = spawn_daemon(&socket, &[]);

        // Journey Stage 1a superseded the old IsADirectory failure:
        // `attach_target` requires the production snapshot-first sequence
        // followed by `InitialTargetResult::Opened`.
        let directory = attach_target(&socket, temp.path(), Path::new("."));

        fs::write(temp.path().join("still-alive.txt"), "alive\n").expect("write survivor");
        let survivor = attach_target(&socket, temp.path(), Path::new("still-alive.txt"));
        assert_eq!(survivor.replica.materialize_string(), "alive\n");

        drop(directory);
        drop(survivor);
        signal_pid(daemon.id(), Signal::SIGTERM);
        assert!(wait_for_exit(&mut daemon, Duration::from_secs(5)).success());
    }

    #[test]
    fn malformed_or_unloadable_targets_fail_closed_without_poisoning_the_daemon() {
        let temp = secure_tempdir();
        let socket = temp.path().join("target-failure.sock");
        let mut daemon = spawn_daemon(&socket, &[]);
        let cwd = temp.path().as_os_str().as_encoded_bytes().to_vec();
        let invalid = [
            (b"relative".to_vec(), b"note".to_vec()),
            (cwd.clone(), Vec::new()),
            (cwd.clone(), b"bad\0name".to_vec()),
            (cwd.clone(), vec![b'x'; 32 * 1024 + 1]),
        ];
        for (index, (bad_cwd, bad_path)) in invalid.into_iter().enumerate() {
            let (frontend_id, mut stream, messages) = open_raw_target(&socket, bad_cwd, bad_path);
            assert_eq!(messages.len(), 1, "failure must send no snapshot");
            match &messages[0] {
                InstanceMessage::InitialTargetResult(InitialTargetResult::Failed { message }) => {
                    assert!(!message.is_empty());
                    assert!(message.len() <= 4 * 1024);
                }
                other => panic!("expected bounded target failure, got {other:?}"),
            }
            assert!(
                read_message::<InstanceMessage>(&mut stream).is_err(),
                "failed bootstrap {index} must close its socket"
            );
            let _ = write_message(
                &mut stream,
                &FrontendEvent::Key(pmacs::protocol::KeyEvent {
                    frontend_id,
                    key: pmacs::protocol::Key::Char('x'),
                    mods: pmacs::protocol::Modifiers::NONE,
                    timestamp_ns: 0,
                }),
            );
        }
        fs::write(temp.path().join("still-alive.txt"), "alive\n").expect("write survivor");
        let survivor = attach_target(&socket, temp.path(), Path::new("still-alive.txt"));
        assert_eq!(survivor.replica.materialize_string(), "alive\n");
        signal_pid(daemon.id(), Signal::SIGTERM);
        assert!(wait_for_exit(&mut daemon, Duration::from_secs(5)).success());
    }

    #[test]
    fn dedup_upgrade_publishes_the_snapshot_to_preexisting_grid_replicas() {
        let temp = secure_tempdir();
        let config_dir = temp.path().join("pmacs");
        fs::create_dir(&config_dir).expect("create config dir");
        let seed_path = temp.path().join("seed.txt");
        let hidden_path = temp.path().join("hidden.txt");
        fs::write(&seed_path, "seed\n").expect("write seed");
        fs::write(&hidden_path, "hidden\n").expect("write hidden");
        fs::write(
            config_dir.join("init.lua"),
            format!(
                "local created_hidden = false\n\
                 pmacs.hook.add('buffer.after-load', function()\n\
                   if created_hidden then return end\n\
                   created_hidden = true\n\
                   pmacs.buffer.find_or_open({hidden_path:?})\n\
                 end)\n"
            ),
        )
        .expect("write hidden-buffer hook");
        let socket = temp.path().join("dedup-upgrade.sock");
        let mut daemon = spawn_daemon(&socket, &[]);
        let (_, mut grid) = attach_surviving_frontend(&socket);

        let seed = attach_target(&socket, temp.path(), Path::new("seed.txt"));
        loop {
            match read_message::<InstanceMessage>(&mut grid).expect("grid seed publication") {
                InstanceMessage::BufferSnapshot { buffer_id, .. }
                    if buffer_id == seed.buffer_id =>
                {
                    break;
                }
                _ => {}
            }
        }

        let hidden = attach_target(&socket, temp.path(), Path::new("hidden.txt"));
        let hidden_snapshot = loop {
            match read_message::<InstanceMessage>(&mut grid).expect("grid hidden publication") {
                InstanceMessage::BufferSnapshot {
                    buffer_id,
                    crdt_snapshot,
                } if buffer_id == hidden.buffer_id => break crdt_snapshot,
                _ => {}
            }
        };
        let replica = CrdtState::new(900).expect("grid hidden replica");
        replica
            .import_snapshot(&hidden_snapshot)
            .expect("import hidden publication");
        assert_eq!(replica.materialize_string(), "hidden\n");

        signal_pid(daemon.id(), Signal::SIGTERM);
        assert!(wait_for_exit(&mut daemon, Duration::from_secs(5)).success());
    }

    #[test]
    fn target_killed_by_hook_fails_closed_and_slow_hook_holds_the_ready_barrier() {
        let temp = secure_tempdir();

        let kill_root = temp.path().join("kill-hook");
        fs::create_dir(&kill_root).expect("create kill hook root");
        fs::set_permissions(&kill_root, fs::Permissions::from_mode(0o700))
            .expect("chmod kill hook root");
        fs::create_dir(kill_root.join("pmacs")).expect("create kill config");
        fs::write(
            kill_root.join("pmacs/init.lua"),
            "pmacs.hook.add('buffer.after-load', function()\n\
             pmacs.buffer.kill(pmacs.window.buffer())\n\
             end)\n",
        )
        .expect("write kill hook");
        let kill_target = kill_root.join("victim.txt");
        fs::write(&kill_target, "victim\n").expect("write victim");
        let kill_socket = kill_root.join("daemon.sock");
        let mut kill_daemon = spawn_daemon(&kill_socket, &[]);
        let failed = request_raw_target(
            &kill_socket,
            kill_root.as_os_str().as_encoded_bytes().to_vec(),
            b"victim.txt".to_vec(),
        );
        assert!(matches!(
            failed.as_slice(),
            [InstanceMessage::InitialTargetResult(InitialTargetResult::Failed { message })]
                if message.contains("removed by a startup hook")
        ));
        let _ = attach_surviving_frontend(&kill_socket);
        signal_pid(kill_daemon.id(), Signal::SIGTERM);
        assert!(wait_for_exit(&mut kill_daemon, Duration::from_secs(5)).success());

        let slow_root = temp.path().join("slow-hook");
        fs::create_dir(&slow_root).expect("create slow hook root");
        fs::set_permissions(&slow_root, fs::Permissions::from_mode(0o700))
            .expect("chmod slow hook root");
        fs::create_dir(slow_root.join("pmacs")).expect("create slow config");
        let marker = slow_root.join("hook-started");
        fs::write(
            slow_root.join("pmacs/init.lua"),
            format!(
                "pmacs.hook.add('buffer.after-load', function()\n\
                 local f = assert(io.open({marker:?}, 'w')); f:write('started'); f:close()\n\
                 os.execute('sleep 1')\n\
                 end)\n"
            ),
        )
        .expect("write slow hook");
        fs::write(slow_root.join("slow.txt"), "slow\n").expect("write slow target");
        let slow_socket = slow_root.join("daemon.sock");
        let mut slow_daemon = spawn_daemon(&slow_socket, &[]);
        let report = slow_root.join("report");
        let fake_daemon = slow_root.join("must-not-spawn");
        let mut probe = ManagedProbe::spawn_target(
            &slow_socket,
            &report,
            &fake_daemon,
            &slow_root,
            &slow_root,
            Path::new("slow.txt"),
        );
        let marker_deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() {
            assert!(
                Instant::now() < marker_deadline,
                "slow hook never reached marker"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !report.exists()
                || parse_report(&report)
                    .get("phase")
                    .is_none_or(|phase| phase != "ready"),
            "frontend reported ready while the startup hook was still blocked"
        );
        let facts = probe.wait_ready();
        assert_eq!(
            facts.get("spawned_daemon").map(String::as_str),
            Some("false")
        );
        assert!(probe.close().success());
        signal_pid(slow_daemon.id(), Signal::SIGTERM);
        assert!(wait_for_exit(&mut slow_daemon, Duration::from_secs(5)).success());
    }

    #[test]
    fn managed_attach_reuses_a_capable_daemon_without_spawning() {
        let temp = secure_tempdir();
        let socket = temp.path().join("existing.sock");
        let report = temp.path().join("report");
        let marker = temp.path().join("spawned");
        let fake_daemon = temp.path().join("fake-daemon");
        write_script(&fake_daemon, "touch \"$PMACS_TEST_MARKER\"");
        let mut daemon = spawn_daemon(&socket, &[]);

        let mut probe = ManagedProbe::spawn(&socket, &report, &fake_daemon, temp.path());
        let facts = probe.wait_ready();
        assert_eq!(
            facts.get("spawned_daemon").map(String::as_str),
            Some("false")
        );
        assert!(!marker.exists());
        assert!(probe.close().success());
        signal_pid(daemon.id(), Signal::SIGTERM);
        assert!(wait_for_exit(&mut daemon, Duration::from_secs(5)).success());
    }

    #[test]
    fn missing_and_stale_sockets_start_real_daemons() {
        for stale in [false, true] {
            let temp = secure_tempdir();
            let socket = temp.path().join("managed.sock");
            if stale {
                let listener = UnixListener::bind(&socket).expect("bind stale socket");
                drop(listener);
                assert!(socket.exists());
            }
            let report = temp.path().join("report");
            let mut probe = ManagedProbe::spawn(&socket, &report, &pmacs_binary(), temp.path());
            let facts = probe.wait_ready();
            assert_eq!(
                facts.get("spawned_daemon").map(String::as_str),
                Some("true")
            );
            assert!(UnixStream::connect(&socket).is_ok());
            let pid = probe.daemon_pid.expect("spawned daemon pid");
            assert!(probe.close().success());
            signal_pid(pid, Signal::SIGTERM);
        }
    }

    #[test]
    fn concurrent_managed_launches_converge_and_reap_the_lock_loser() {
        let temp = secure_tempdir();
        let socket = temp.path().join("race.sock");
        let first_ready = temp.path().join("first-ready");
        let second_ready = temp.path().join("second-ready");
        let first_wrapper = temp.path().join("first-daemon");
        let second_wrapper = temp.path().join("second-daemon");
        let wrapper = "touch \"$PMACS_BARRIER_SELF\"\n\
                       while [ ! -e \"$PMACS_BARRIER_PEER\" ]; do sleep 0.01; done\n\
                       exec \"$PMACS_REAL_DAEMON\" \"$@\"";
        write_script(&first_wrapper, wrapper);
        write_script(&second_wrapper, wrapper);
        let real_daemon = pmacs_binary();

        let mut first = ManagedProbe::spawn_with_env(
            &socket,
            &temp.path().join("first-report"),
            &first_wrapper,
            temp.path(),
            &[
                ("PMACS_BARRIER_SELF", &first_ready),
                ("PMACS_BARRIER_PEER", &second_ready),
                ("PMACS_REAL_DAEMON", &real_daemon),
            ],
        );
        let mut second = ManagedProbe::spawn_with_env(
            &socket,
            &temp.path().join("second-report"),
            &second_wrapper,
            temp.path(),
            &[
                ("PMACS_BARRIER_SELF", &second_ready),
                ("PMACS_BARRIER_PEER", &first_ready),
                ("PMACS_REAL_DAEMON", &real_daemon),
            ],
        );
        let first_facts = first.wait_ready();
        let second_facts = second.wait_ready();
        assert_eq!(
            first_facts.get("spawned_daemon").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            second_facts.get("spawned_daemon").map(String::as_str),
            Some("true")
        );
        assert!(UnixStream::connect(&socket).is_ok());

        let deadline = Instant::now() + Duration::from_secs(5);
        let first_lost = loop {
            let first_reaped = parse_report(&first.report)
                .get("daemon_reaped")
                .is_some_and(|value| value == "true");
            let second_reaped = parse_report(&second.report)
                .get("daemon_reaped")
                .is_some_and(|value| value == "true");
            if first_reaped ^ second_reaped {
                break first_reaped;
            }
            assert!(
                Instant::now() < deadline,
                "exactly one losing daemon child was not reaped"
            );
            thread::sleep(Duration::from_millis(20));
        };

        if first_lost {
            assert!(first.close().success());
            assert!(second.close().success());
        } else {
            assert!(second.close().success());
            assert!(first.close().success());
        }
    }

    #[test]
    fn ctrl_c_on_launcher_group_does_not_reach_spawned_daemon() {
        let temp = secure_tempdir();
        let socket = temp.path().join("signal.sock");
        let report = temp.path().join("signal-report");
        let wrapper = temp.path().join("headless-gpu-wrapper");
        write_script(
            &wrapper,
            "exec \"$PMACS_REAL_GPU\" --headless-managed-probe \"$2\" \"$PMACS_REPORT\" \"$3\"",
        );

        let mut command = Command::new(pmacs_binary());
        command
            .args(["--gpu", "--socket"])
            .arg(&socket)
            .env(TEST_GPU_OVERRIDE, &wrapper)
            .env("PMACS_REAL_GPU", gpu_binary())
            .env("PMACS_REPORT", &report)
            .env("HOME", temp.path())
            .env("XDG_CONFIG_HOME", temp.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.process_group(0);
        let mut launcher = command.spawn().expect("spawn launcher process group");
        let facts = wait_for_fact(&report, "phase", "ready", Duration::from_secs(10));
        let daemon_pid = facts["daemon_pid"].parse::<u32>().expect("daemon pid");
        let (survivor_id, mut survivor) = attach_surviving_frontend(&socket);

        kill(Pid::from_raw(-launcher.id().cast_signed()), Signal::SIGINT)
            .expect("signal launcher group");
        let _ = wait_for_exit(&mut launcher, Duration::from_secs(5));

        write_message(
            &mut survivor,
            &FrontendEvent::Resize {
                frontend_id: survivor_id,
                size: CellSize::new(31, 91),
            },
        )
        .expect("resize surviving frontend after launcher Ctrl-C");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                Instant::now() < deadline,
                "pre-signal frontend did not render after launcher Ctrl-C"
            );
            if matches!(
                read_message::<InstanceMessage>(&mut survivor)
                    .expect("read surviving frontend after launcher Ctrl-C"),
                InstanceMessage::CellDelta {
                    full_grid: true,
                    ..
                }
            ) {
                break;
            }
        }
        signal_pid(daemon_pid, Signal::SIGTERM);
    }

    #[test]
    fn capability_and_protocol_mismatches_never_spawn_replacements() {
        let temp = secure_tempdir();
        let marker = temp.path().join("spawned");
        let fake_daemon = temp.path().join("fake-daemon");
        write_script(&fake_daemon, "touch \"$PMACS_TEST_MARKER\"");

        let capability_socket = temp.path().join("capability.sock");
        let mut daemon = spawn_daemon(
            &capability_socket,
            &[
                ("PMACS_INSTANCE_CRDT_REPLICA", "0"),
                ("PMACS_INSTANCE_SEMANTIC_RENDER", "0"),
            ],
        );
        let capability_report = temp.path().join("capability-report");
        let output = Command::new(gpu_binary())
            .args(["--headless-managed-probe"])
            .arg(&capability_socket)
            .arg(&capability_report)
            .arg(&fake_daemon)
            .env("PMACS_TEST_MARKER", &marker)
            .output()
            .expect("run capability mismatch probe");
        assert!(!output.status.success());
        assert!(
            fs::read_to_string(&capability_report)
                .unwrap()
                .contains("required capabilities")
        );
        assert!(!marker.exists());
        assert!(daemon.try_wait().expect("inspect daemon").is_none());
        signal_pid(daemon.id(), Signal::SIGTERM);
        let _ = daemon.wait();

        let protocol_socket = temp.path().join("protocol.sock");
        let listener = UnixListener::bind(&protocol_socket).expect("bind protocol fixture");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept protocol fixture");
            let hello = Hello {
                protocol_version: PROTOCOL_VERSION + 100,
                assigned_frontend_id: FrontendId::LOCAL,
                instance_identity: InstanceIdentity {
                    pmacs_version: "protocol-fixture".to_owned(),
                    build_hash: None,
                    instance_name: None,
                    uptime_secs: 0,
                    working_directory: "/tmp".to_owned(),
                },
                instance_capabilities: InstanceCapabilities {
                    multi_frontend: true,
                    crdt_replica: true,
                    semantic_render: true,
                },
            };
            write_message(&mut stream, &hello).expect("write mismatched Hello");
        });
        let protocol_report = temp.path().join("protocol-report");
        let output = Command::new(gpu_binary())
            .args(["--headless-managed-probe"])
            .arg(&protocol_socket)
            .arg(&protocol_report)
            .arg(&fake_daemon)
            .env("PMACS_TEST_MARKER", &marker)
            .output()
            .expect("run protocol mismatch probe");
        server.join().expect("protocol fixture");
        assert!(!output.status.success());
        assert!(
            fs::read_to_string(&protocol_report)
                .unwrap()
                .contains("protocol version")
        );
        assert!(!marker.exists());
    }

    #[test]
    fn bounded_startup_failure_reports_child_status() {
        let temp = secure_tempdir();
        let socket = temp.path().join("never.sock");
        let report = temp.path().join("failure-report");
        let failing_daemon = temp.path().join("failing-daemon");
        write_script(&failing_daemon, "exit 17");
        let start = Instant::now();
        let output = Command::new(gpu_binary())
            .args(["--headless-managed-probe"])
            .arg(&socket)
            .arg(&report)
            .arg(&failing_daemon)
            .output()
            .expect("run bounded failure probe");
        assert!(!output.status.success());
        assert!(start.elapsed() >= Duration::from_secs(4));
        assert!(start.elapsed() < Duration::from_secs(8));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("exit status: 17"),
            "unexpected stderr: {stderr}"
        );
    }

    #[test]
    fn managed_probe_observes_disconnect_and_reaps_daemon_child() {
        let temp = secure_tempdir();
        let socket = temp.path().join("reap.sock");
        let report = temp.path().join("reap-report");
        let mut probe = ManagedProbe::spawn(&socket, &report, &pmacs_binary(), temp.path());
        probe.wait_ready();
        let daemon_pid = probe.daemon_pid.expect("daemon pid");
        signal_pid(daemon_pid, Signal::SIGTERM);
        let facts = wait_for_fact(&report, "daemon_reaped", "true", Duration::from_secs(5));
        assert!(!facts["disconnect"].is_empty());
        assert!(probe.close().success());
        let final_facts = parse_report(&report);
        assert_eq!(
            final_facts.get("phase").map(String::as_str),
            Some("complete")
        );
        assert_eq!(
            final_facts.get("daemon_reaped").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn gpu_cli_help_version_and_invalid_argv_are_headless_and_strict() {
        let help = Command::new(gpu_binary())
            .arg("--help")
            .output()
            .expect("GPU help");
        assert!(help.status.success());
        let help_text = String::from_utf8_lossy(&help.stdout);
        assert!(help_text.contains("pmacs --gpu"));
        assert!(help_text.contains("ADVANCED DIRECT ATTACH"));

        let version = Command::new(gpu_binary())
            .arg("--version")
            .output()
            .expect("GPU version");
        assert!(version.status.success());
        assert!(String::from_utf8_lossy(&version.stdout).contains("protocol v"));

        let bare = Command::new(gpu_binary())
            .output()
            .expect("bare GPU invocation");
        assert_eq!(bare.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&bare.stderr).contains("pmacs --gpu"));

        let help_extra = Command::new(gpu_binary())
            .args(["--help", "extra"])
            .output()
            .expect("GPU help with extra operand");
        assert_eq!(help_extra.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&help_extra.stderr).contains("does not accept operands"));

        for argv in [
            vec!["--attach"],
            vec!["--attach", "/tmp/x.sock", "ignored"],
            vec!["--attach", "--help"],
            vec!["unexpected"],
        ] {
            let output = Command::new(gpu_binary())
                .args(&argv)
                .output()
                .expect("invalid GPU CLI");
            assert_eq!(output.status.code(), Some(2), "accepted argv {argv:?}");
        }
    }
}
