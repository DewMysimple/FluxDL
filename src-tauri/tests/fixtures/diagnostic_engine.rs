//! Synthetic, deterministic protocol engine. NEVER distributed as yt-dlp.
use std::{io::Write, path::PathBuf, thread::sleep, time::Duration};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("2026.11.09-fixture");
        return;
    }
    if args.iter().any(|a| a == "--help") {
        println!("--print [WHEN:]TEMPLATE before_dl --no-simulate --progress");
        return;
    }
    let url = args.last().unwrap();
    let name = url.rsplit('/').next().unwrap().split('?').next().unwrap();
    if args.iter().any(|a| a == "--dump-single-json") {
        sleep(Duration::from_millis(60));
        if name.contains("parse-fail") {
            eprintln!("ERROR: fixture parse failure https://private.invalid/private?sig=FIXTURE_SIG Cookie: FIXTURE_COOKIE");
            std::process::exit(1);
        }
        if name.contains("recheck-fail") {
            let marker = std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .join(format!("{name}.marker"));
            if marker.exists() {
                eprintln!("ERROR: fixture recheck failure");
                std::process::exit(1);
            }
            std::fs::write(marker, b"first parse").unwrap();
        }
        println!(
            r#"{{"id":"fixture","title":"PRIVATE_TITLE_FIXTURE","extractor_key":"Fixture","duration":1,"formats":[{{"format_id":"direct","ext":"mp4","height":360,"acodec":"aac","vcodec":"h264"}}]}}"#
        );
        return;
    }
    if name.contains("download-fail") {
        eprintln!("ERROR: download failure FIXTURE_TOKEN C:\\Users\\PrivatePerson\\secret.mp4");
        std::process::exit(1);
    }
    if name.contains("silent") { sleep(Duration::from_millis(40)); return; }
    if name.contains("stderr-only") { eprintln!("ERROR: fixed stderr-only fixture"); std::process::exit(1); }
    if name.contains("immediate-output") { println!("fixed immediate fixture activity"); std::io::stdout().flush().unwrap(); }
    if name.contains("stderr-first") { eprintln!("fixed fixture activity"); std::io::stderr().flush().unwrap(); }
    sleep(Duration::from_millis(if name.contains("delayed-output") { 180 } else { 35 }));
    for pair in args.windows(2) { if pair[0] == "--print" { if let Some(template) = pair[1].strip_prefix("before_dl:") { println!("{}",template.replace("%%","%"));std::io::stdout().flush().unwrap(); } } }
    sleep(Duration::from_millis(65));
    if name.contains("marker-delayed-progress") { sleep(Duration::from_millis(150)); }
    if name.contains("slow") {
        sleep(Duration::from_millis(300));
    }
    if !name.contains("no-progress") {
        for n in 1..=4 {
            println!("__FLUX_PROGRESS__{{\"status\":\"downloading\",\"downloaded_bytes\":{},\"total_bytes\":400}}",n*100);
            std::io::stdout().flush().unwrap();
            sleep(Duration::from_millis(20));
        }
    }
    let directory = PathBuf::from(&args[args.iter().position(|a| a == "--paths").unwrap() + 1]);
    let path = directory.join(format!("{name}.mp4"));
    std::fs::write(&path, b"synthetic fixture only").unwrap();
    if name.contains("postprocess") {
        println!("__FLUX_POST__finished");
        std::io::stdout().flush().unwrap();
        sleep(Duration::from_millis(80));
    }
    let encoded = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    println!("__FLUX_FILE__\"{encoded}\"");
}
