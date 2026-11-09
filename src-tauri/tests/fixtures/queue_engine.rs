//! Deterministic test engine. It is NEVER bundled or used by the production app.
//! Compile with scripts/test-queue.ps1; public-media verification is separate.
use std::{io::Write, path::PathBuf, time::Duration};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("2026.01.01-fixture");
        return;
    }
    let url = args.last().expect("URL argument");
    let name = url.rsplit('/').next().unwrap_or("fixture");
    if args.iter().any(|a| a == "--dump-single-json") {
        println!(
            r#"{{"id":"{name}","title":"Fixture {name}","extractor_key":"Fixture","duration":5,"formats":[{{"format_id":"direct","ext":"mp4","height":360,"acodec":"aac","vcodec":"h264"}}]}}"#
        );
        return;
    }
    let index = args
        .iter()
        .position(|a| a == "--paths")
        .expect("output directory");
    let directory = PathBuf::from(&args[index + 1]);
    let mut events = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("events.log"))
        .unwrap();
    writeln!(events, "start {name}").unwrap();
    events.flush().unwrap();
    if name == "fail-first" && !directory.join("failure-marker").exists() {
        std::fs::write(directory.join("failure-marker"), b"fixture").unwrap();
        eprintln!("ERROR: network unavailable (intentional fixture failure)");
        std::process::exit(1);
    }
    let steps = if name == "slow" { 200 } else { 15 };
    for index in 1..=steps {
        println!("__FLUX_PROGRESS__{{\"downloaded_bytes\":{},\"total_bytes\":{},\"speed\":2000,\"eta\":1}}",index*100,steps*100);
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_millis(50));
    }
    let path = directory.join(format!("{name}.mp4"));
    std::fs::write(&path, b"deterministic fixture media, not a real video").unwrap();
    let encoded = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    println!("__FLUX_POST__finished");
    println!("__FLUX_FILE__\"{encoded}\"");
    writeln!(events, "finish {name}").unwrap();
}
