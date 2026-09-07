//! gw2ei-cli:EI 等价 CLI —— 解析日志并输出对齐官方的 JSON(compact)。
//! Usage: gw2ei-cli <log.zevtc|log.evtc> [-o out.json] [--content-dir DIR]

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut log_path: Option<PathBuf> = None;
    let mut out_path: Option<PathBuf> = None;
    let mut content_dir: PathBuf = PathBuf::from("content");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => out_path = it.next().map(PathBuf::from),
            "--content-dir" => content_dir = it.next().map(PathBuf::from).unwrap_or(content_dir),
            "-h" | "--help" => {
                println!("usage: gw2ei-cli <log.zevtc> [-o out.json] [--content-dir DIR]");
                std::process::exit(0);
            }
            _ => log_path = Some(PathBuf::from(a)),
        }
    }
    let Some(log_path) = log_path else {
        eprintln!("error: missing log path");
        std::process::exit(1);
    };
    if !log_path.exists() {
        eprintln!("error: log not found: {}", log_path.display());
        std::process::exit(1);
    }
    match gw2ei_json::parse_and_build(&log_path, &content_dir, &gw2ei_json::BuildOptions::default()) {
        Ok(json) => {
            let value = gw2ei_json::shorten_numbers(serde_json::to_value(&json).expect("to value"));
            let text = serde_json::to_string(&value).expect("serialize");
            match out_path {
                Some(p) => {
                    if let Err(e) = std::fs::write(&p, &text) {
                        eprintln!("error: write {}: {e}", p.display());
                        std::process::exit(1);
                    }
                    println!("Processed - {}", p.display());
                }
                None => println!("{text}"),
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
