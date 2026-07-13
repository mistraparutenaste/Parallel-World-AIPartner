use std::{
    thread,
    time::{Duration, Instant},
};

use pw_platform::process::{ProcessSpec, ProcessSupervisor, SupervisorError};

fn helper(mode: &str) -> ProcessSpec {
    let mut spec = ProcessSpec::new(std::env::current_exe().unwrap());
    spec.args = vec![
        "--exact".into(),
        "supervisor_helper_child".into(),
        "--nocapture".into(),
    ];
    spec.env.push(("PW_HELPER_MODE".into(), mode.into()));
    spec.output_capacity = 4096;
    spec
}

#[test]
#[allow(
    clippy::zombie_processes,
    reason = "the supervisor tree-kill test owns and reaps this descendant"
)]
fn supervisor_helper_child() {
    let Ok(mode) = std::env::var("PW_HELPER_MODE") else {
        return;
    };
    match mode.as_str() {
        "instant" => std::process::exit(17),
        "delayed" => {
            thread::sleep(Duration::from_millis(100));
            std::process::exit(0);
        }
        "flood" => {
            for _ in 0..20_000 {
                eprintln!("stderr flood payload");
            }
        }
        "hang" => loop {
            thread::sleep(Duration::from_secs(1));
        },
        "descendant" => {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", "supervisor_helper_child", "--nocapture"])
                .env("PW_HELPER_MODE", "hang");
            let child = child.spawn().unwrap();
            eprintln!("DESCENDANT_PID={}", child.id());
            loop {
                thread::sleep(Duration::from_secs(1));
            }
        }
        _ => std::process::exit(2),
    }
}

#[test]
fn detects_instant_and_delayed_exit() {
    let supervisor = ProcessSupervisor::spawn(&helper("instant")).unwrap();
    let exit = supervisor
        .wait_for_exit(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert_eq!(exit.code(), Some(17));

    let delayed = ProcessSupervisor::spawn(&helper("delayed")).unwrap();
    assert!(delayed.try_wait().unwrap().is_none());
    assert!(
        delayed
            .wait_for_exit(Duration::from_secs(2))
            .unwrap()
            .is_some()
    );
}

#[test]
fn drains_stderr_without_unbounded_memory_or_deadlock() {
    let supervisor = ProcessSupervisor::spawn(&helper("flood")).unwrap();
    assert!(
        supervisor
            .wait_for_exit(Duration::from_secs(5))
            .unwrap()
            .is_some()
    );
    let output = supervisor.output();
    assert!(output.stderr.len() <= 4096);
    assert!(output.stderr_dropped > 0);
}

#[test]
fn stop_waits_then_kills_and_reaps_hung_child() {
    let supervisor = ProcessSupervisor::spawn(&helper("hang")).unwrap();
    let pid = supervisor.pid();
    let started = Instant::now();
    supervisor.stop(Duration::from_millis(100)).unwrap();
    assert!(started.elapsed() >= Duration::from_millis(80));
    assert!(supervisor.try_wait().unwrap().is_some());
    assert_eq!(supervisor.pid(), pid);
}

#[test]
fn spawn_failure_and_stop_race_are_safe() {
    let missing = ProcessSpec::new("definitely-not-a-real-parallel-world-executable");
    assert!(matches!(
        ProcessSupervisor::spawn(&missing),
        Err(SupervisorError::InvalidExecutable(_))
    ));

    let supervisor = std::sync::Arc::new(ProcessSupervisor::spawn(&helper("hang")).unwrap());
    let other = supervisor.clone();
    let join = thread::spawn(move || other.stop(Duration::from_millis(50)));
    supervisor.stop(Duration::from_millis(50)).unwrap();
    join.join().unwrap().unwrap();
}

#[test]
fn stale_generation_cannot_stop_replacement() {
    let first = ProcessSupervisor::spawn(&helper("hang")).unwrap();
    let stale = first.generation();
    first.stop(Duration::from_millis(10)).unwrap();
    let replacement = first.restart(&helper("hang")).unwrap();
    assert!(
        !replacement
            .stop_generation(stale, Duration::from_millis(10))
            .unwrap()
    );
    assert!(replacement.try_wait().unwrap().is_none());
    replacement.stop(Duration::from_millis(10)).unwrap();
}

#[test]
fn health_probe_reports_running_child() {
    let supervisor = ProcessSupervisor::spawn(&helper("hang")).unwrap();
    assert!(supervisor.is_healthy().unwrap());
    supervisor.stop(Duration::from_millis(10)).unwrap();
    assert!(!supervisor.is_healthy().unwrap());
}

#[cfg(windows)]
#[test]
fn stop_reaps_descendant_process_tree() {
    let supervisor = ProcessSupervisor::spawn(&helper("descendant")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let descendant = loop {
        let stderr = String::from_utf8_lossy(&supervisor.output().stderr).into_owned();
        if let Some(pid) = stderr
            .lines()
            .find_map(|line| line.strip_prefix("DESCENDANT_PID="))
        {
            break pid.parse::<u32>().unwrap();
        }
        assert!(Instant::now() < deadline, "descendant pid was not reported");
        thread::sleep(Duration::from_millis(10));
    };
    supervisor.stop(Duration::from_millis(50)).unwrap();
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {descendant}"), "/NH"])
        .output()
        .unwrap();
    let listing = String::from_utf8_lossy(&output.stdout);
    assert!(
        !listing.contains(&descendant.to_string()),
        "orphan process: {listing}"
    );
}
