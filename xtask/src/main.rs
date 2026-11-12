mod verify_live;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};

const NATS_HEALTH: &str = "http://127.0.0.1:8222/healthz?js-enabled-only=true";
const CLICKHOUSE_URL: &str = "http://127.0.0.1:8123";
const CLICKHOUSE_USER: &str = "orderflow";
const CLICKHOUSE_PASSWORD: &str = "orderflow";
const PACKAGE_ENV: [&str; 6] = [
    "CARGO_MANIFEST_DIR",
    "CARGO_MANIFEST_PATH",
    "CARGO_MANIFEST_LINKS",
    "CARGO_CRATE_NAME",
    "CARGO_BIN_NAME",
    "CARGO_PRIMARY_PACKAGE",
];
const LIVE_PROFILE: &str = "live";
const LIVE_SERVICES: [&str; 5] = [
    "collector",
    "aggregator",
    "book-engine",
    "writer",
    "gateway",
];
const LIVE_IMAGE: &str = "orderflow-services:local";
const GATEWAY_HEALTH: &str = "http://127.0.0.1:8080/healthz";
const CHAOS_PROFILE: &str = "chaos";
const CHAOS_CLICKHOUSE: &str = "clickhouse-chaos";
const TEST_BUILD_ATTEMPTS: u32 = if cfg!(windows) { 5 } else { 1 };

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("up") => up(),
        Some("down") => down(args.iter().any(|a| a == "--volumes")),
        Some("smoke") => smoke(),
        Some("ci") => ci(),
        Some("web") => web(),
        Some("live") => live(args.get(1).map(String::as_str), &args[2..]),
        Some("verify-live") => {
            verify_live::Options::parse(&args[1..]).and_then(|options| verify_live::run(&options))
        }
        _ => {
            eprintln!(
                "usage: cargo xtask <up | down [--volumes] | smoke | ci | web                  | live <up | down | logs [service..] | status>                  | verify-live [--gateway URL] [--minutes N]>"
            );
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}

fn compose() -> Command {
    let mut cmd = Command::new("docker");
    cmd.current_dir(workspace_root())
        .args(["compose", "--file", "deploy/compose.yaml"]);
    cmd
}

fn cargo() -> Command {
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(workspace_root());
    for (name, _) in std::env::vars_os() {
        if name
            .to_str()
            .is_some_and(|name| PACKAGE_ENV.contains(&name) || name.starts_with("CARGO_PKG_"))
        {
            cmd.env_remove(&name);
        }
    }
    cmd
}

fn pnpm(args: &[&str]) -> Command {
    let mut cmd = Command::new(if cfg!(windows) { "pnpm.cmd" } else { "pnpm" });
    cmd.current_dir(workspace_root().join("web")).args(args);
    cmd
}

fn web() -> Result<()> {
    run(pnpm(&["install", "--frozen-lockfile"]))?;
    run(pnpm(&["run", "build"]))
}

fn run(mut cmd: Command) -> Result<()> {
    let shown = format!("{cmd:?}");
    let status = cmd
        .status()
        .with_context(|| format!("failed to spawn {shown}"))?;
    if !status.success() {
        bail!("{shown} exited with {status}");
    }
    Ok(())
}

fn up() -> Result<()> {
    let mut cmd = compose();
    cmd.args(["up", "--detach", "--wait"]);
    run(cmd)?;
    smoke()
}

fn down(volumes: bool) -> Result<()> {
    let mut cmd = compose();
    cmd.args([
        "--profile",
        LIVE_PROFILE,
        "--profile",
        CHAOS_PROFILE,
        "down",
    ]);
    if volumes {
        cmd.arg("--volumes");
    }
    run(cmd)
}

fn live(action: Option<&str>, rest: &[String]) -> Result<()> {
    let compose_live = |args: &[&str]| {
        let mut cmd = compose();
        cmd.args(["--profile", LIVE_PROFILE]).args(args);
        cmd
    };
    match action {
        Some("up") => {
            smoke().context("the infrastructure is not up; run `cargo xtask up` first")?;
            let mut build = Command::new("docker");
            build.current_dir(workspace_root()).args([
                "build",
                "--file",
                "deploy/Dockerfile",
                "--tag",
                LIVE_IMAGE,
                ".",
            ]);
            run(build)?;
            let mut up = vec!["up", "--detach", "--wait", "--no-build"];
            up.extend(LIVE_SERVICES);
            run(compose_live(&up))?;
            wait_until_healthy(GATEWAY_HEALTH, std::time::Duration::from_secs(30))?;
            println!("ok  live services up; gateway on http://127.0.0.1:8080");
            Ok(())
        }
        Some("down") => {
            // Only the services: NATS, ClickHouse and their data stay.
            let mut stop = vec!["stop"];
            stop.extend(LIVE_SERVICES);
            run(compose_live(&stop))?;
            let mut rm = vec!["rm", "--force"];
            rm.extend(LIVE_SERVICES);
            run(compose_live(&rm))
        }
        Some("logs") => {
            let mut logs = vec!["logs", "--timestamps", "--tail", "200"];
            logs.extend(rest.iter().map(String::as_str));
            run(compose_live(&logs))
        }
        Some("status") => run(compose_live(&["ps", "--all"])),
        _ => bail!("usage: cargo xtask live <up | down | logs [service..] | status>"),
    }
}

fn wait_until_healthy(url: &str, timeout: std::time::Duration) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match ureq::get(url).call() {
            Ok(_) => return Ok(()),
            Err(err) if std::time::Instant::now() >= deadline => {
                return Err(err).with_context(|| format!("{url} not healthy after {timeout:?}"));
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
        }
    }
}

fn smoke() -> Result<()> {
    ureq::get(NATS_HEALTH)
        .call()
        .context("NATS is not healthy with JetStream enabled")?;
    println!("ok  nats      jetstream enabled");

    let answer = ureq::post(format!("{CLICKHOUSE_URL}/?database=orderflow"))
        .header("X-ClickHouse-User", CLICKHOUSE_USER)
        .header("X-ClickHouse-Key", CLICKHOUSE_PASSWORD)
        .send("SELECT 1")
        .context("ClickHouse did not answer")?
        .body_mut()
        .read_to_string()?;
    if answer.trim() != "1" {
        bail!("ClickHouse answered `SELECT 1` with {answer:?}");
    }
    println!("ok  clickhouse database `orderflow` reachable");
    Ok(())
}

fn ci() -> Result<()> {
    let mut fmt = cargo();
    fmt.args(["fmt", "--all", "--check"]);
    run(fmt)?;

    let mut clippy = cargo();
    clippy.args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "--deny",
        "warnings",
    ]);
    run(clippy)?;

    run(pnpm(&["install", "--frozen-lockfile"]))?;
    run(pnpm(&["run", "test"]))?;
    run(pnpm(&["run", "build"]))?;

    smoke().context("the stack is not up; run `cargo xtask up` first")?;

    build_tests()?;
    let mut test = cargo();
    test.args(["test", "--workspace"]);
    run(test)?;

    let mut chaos = compose();
    chaos.args([
        "--profile",
        CHAOS_PROFILE,
        "up",
        "--detach",
        "--wait",
        CHAOS_CLICKHOUSE,
    ]);
    run(chaos)?;
    let mut disruptive = cargo();
    disruptive.args(["test", "--workspace", "--", "--ignored", "--test-threads=1"]);
    let result = run(disruptive);
    let mut remove = compose();
    remove.args([
        "--profile",
        CHAOS_PROFILE,
        "rm",
        "--stop",
        "--force",
        "--volumes",
        CHAOS_CLICKHOUSE,
    ]);
    result.and(run(remove))
}

fn build_tests() -> Result<()> {
    let mut attempt = 1;
    loop {
        let mut build = cargo();
        build.args(["build", "--workspace", "--tests", "--keep-going"]);
        match run(build) {
            Err(err) if attempt < TEST_BUILD_ATTEMPTS => {
                eprintln!(
                    "xtask: building tests failed ({err:#}); retrying {attempt}/{}, as                      executables locked by a scanner are relinked on the next attempt",
                    TEST_BUILD_ATTEMPTS - 1
                );
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            result => return result,
        }
    }
}
