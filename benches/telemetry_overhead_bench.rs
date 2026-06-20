#[path = "common/bench_config.rs"]
mod bench_config;
#[path = "common/binary.rs"]
mod binary;
#[path = "common/real_project.rs"]
mod real_project;
#[path = "common/sandbox_create.rs"]
mod sandbox_create;

use bench_config::real_project_criterion_config;
use binary::resolve_release_binary;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use real_project::require_real_project_dir;
use sandbox_create::create_indexed_workspace;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::Duration;

const STATUS_REQUEST: &str = r#"{"request_id":"telemetry-status","command":"status"}"#;
const QUERY_REQUEST: &str = r#"{"request_id":"telemetry-query","command":"explain_condition","target":"comp:app/销售.app/销售/合同协议.spg|input3","budget":"compact","intent":"writer"}"#;

/// 长驻 stdio server 客户端，用于 warm 协议往返测量。
struct WarmStdioClient {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
}

impl WarmStdioClient {
    fn spawn(binary: &Path, trace_mode: &str, db_path: &Path, project_dir: &Path) -> Self {
        let mut child = Command::new(binary)
            .arg("--serve-stdio")
            .arg("--graph-db-path")
            .arg(db_path)
            .arg("--project-dir")
            .arg(project_dir)
            .arg("--trace")
            .arg(trace_mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn warm stdio server");
        let stdin = child.stdin.take().expect("stdio server stdin");
        let stdout = child.stdout.take().expect("stdio server stdout");
        let mut client = Self {
            child,
            stdin,
            reader: BufReader::new(stdout),
        };
        let _ = client.roundtrip(STATUS_REQUEST);
        client
    }

    fn roundtrip(&mut self, line: &str) -> String {
        writeln!(self.stdin, "{line}").expect("write stdio request");
        self.stdin.flush().expect("flush stdio request");
        let mut response = String::new();
        self.reader
            .read_line(&mut response)
            .expect("read stdio response");
        response
    }
}

impl Drop for WarmStdioClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bench_stdio_roundtrip(
    c: &mut Criterion,
    bench_name: &str,
    trace_mode: &str,
    db_path: &Path,
    project_dir: &Path,
    request_line: &str,
) {
    let binary = resolve_release_binary();
    c.bench_function(bench_name, |bench| {
        let mut client = WarmStdioClient::spawn(&binary, trace_mode, db_path, project_dir);
        bench.iter(|| {
            let response = client.roundtrip(black_box(request_line));
            assert!(
                response.contains("\"ok\":true"),
                "warm stdio response should succeed: {response}"
            );
            black_box(response);
        });
    });
}

fn bench_telemetry_overhead(c: &mut Criterion) {
    let Some(source_project_dir) = require_real_project_dir("telemetry_overhead_bench") else {
        return;
    };
    let workspace = create_indexed_workspace("telemetry-overhead", &source_project_dir)
        .expect("create workspace");
    if !resolve_release_binary().exists() {
        eprintln!(
            "skip telemetry_overhead_bench: build binary first with \
             `cargo build --profile release-fast --features telemetry`"
        );
        return;
    }

    bench_stdio_roundtrip(
        c,
        "telemetry_stdio_status_trace_off",
        "off",
        &workspace.db_path,
        &workspace.project_dir,
        STATUS_REQUEST,
    );
    bench_stdio_roundtrip(
        c,
        "telemetry_stdio_query_trace_off",
        "off",
        &workspace.db_path,
        &workspace.project_dir,
        QUERY_REQUEST,
    );
    bench_stdio_roundtrip(
        c,
        "telemetry_stdio_query_trace_json",
        "json",
        &workspace.db_path,
        &workspace.project_dir,
        QUERY_REQUEST,
    );

    if std::env::var("METADATA_CHECKER_OTLP_ENDPOINT").is_ok() {
        bench_stdio_roundtrip(
            c,
            "telemetry_stdio_query_trace_otlp",
            "otlp",
            &workspace.db_path,
            &workspace.project_dir,
            QUERY_REQUEST,
        );
    } else {
        eprintln!(
            "skip telemetry_stdio_query_trace_otlp: set METADATA_CHECKER_OTLP_ENDPOINT to enable"
        );
    }

    // 避免子进程 stderr 管道阻塞。
    std::thread::sleep(Duration::from_millis(50));
}

criterion_group! {
    name = telemetry_overhead_benches;
    config = real_project_criterion_config();
    targets = bench_telemetry_overhead
}
criterion_main!(telemetry_overhead_benches);
