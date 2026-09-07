//! `.zevtc` / `.evtc.zip` 容器读取。对齐 C# `EvtcParser.ParseLog(FileInfo)`
//! （EvtcParser.cs:57-76）：压缩包内**恰 1 个 entry**，否则
//! `EvtcFileException("Invalid Archive")`；解压入内存后检查 TooBig。
//!
//! 与 SBR `evtc-format` 的三态条目选择不同（SBR 支持具名 `.evtc` 条目优先 +
//! 单条目兜底 + 内容探测），EI 严格要求 entry 数 == 1 —— 有意偏差，记录于
//! `design.md` 的 C# 对齐点清单。

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::error::EvtcError;
use crate::models::ParserSettings;

pub(crate) fn read_compressed_log(path: &Path, settings: &ParserSettings) -> Result<Vec<u8>, EvtcError> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // C# `arch.Entries.Count != 1`（EvtcParser.cs:61-64）。目录条目也计入
    // `ZipArchive.Entries`（.NET 的 ZipArchive 会把目录作为 entry 暴露，
    // 与 `file_names()` 的过滤行为不同），因此用条目总数直接比较。
    if archive.len() != 1 {
        return Err(EvtcError::InvalidArchive);
    }

    let mut entry = archive.by_index(0)?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;

    // C# `operation.SetFileSize(ms.Length / (1024L * 1024L))` 后与
    // `TooBigLimit`（下限 100MB）比较（EvtcParser.cs:69-73）。
    let size_mb = bytes.len() as u64 / (1024 * 1024);
    if size_mb > settings.too_big_limit_mb {
        return Err(EvtcError::TooBig {
            size: size_mb,
            limit: settings.too_big_limit_mb,
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use super::*;

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .expect("start entry");
            writer.write_all(content).expect("write entry");
        }
        writer.finish().expect("finish zip").into_inner()
    }

    #[test]
    fn zip_with_exactly_one_entry_ok() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("gw2ei_zip_ok_{}.zevtc", std::process::id()));
        std::fs::write(&path, make_zip(&[("20260530-205048", b"EVTC20260507")]))
            .expect("write temp zip");
        let result = read_compressed_log(&path, &ParserSettings::default());
        std::fs::remove_file(&path).expect("cleanup");
        assert_eq!(result.expect("single entry must parse"), b"EVTC20260507");
    }

    #[test]
    fn zip_with_two_entries_is_invalid_archive() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("gw2ei_zip_two_{}.zevtc", std::process::id()));
        std::fs::write(
            &path,
            make_zip(&[
                ("one", b"EVTC20260507"),
                ("two", b"EVTC20260507"),
            ]),
        )
        .expect("write temp zip");
        let result = read_compressed_log(&path, &ParserSettings::default());
        std::fs::remove_file(&path).expect("cleanup");
        assert!(matches!(result, Err(EvtcError::InvalidArchive)));
    }

    #[test]
    fn zip_with_zero_entries_is_invalid_archive() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("gw2ei_zip_empty_{}.zevtc", std::process::id()));
        std::fs::write(&path, make_zip(&[])).expect("write temp zip");
        let result = read_compressed_log(&path, &ParserSettings::default());
        std::fs::remove_file(&path).expect("cleanup");
        assert!(matches!(result, Err(EvtcError::InvalidArchive)));
    }

    #[test]
    fn non_zip_bytes_are_zip_error() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("gw2ei_zip_garbage_{}.zevtc", std::process::id()));
        std::fs::write(&path, b"this is not a zip archive").expect("write temp file");
        let result = read_compressed_log(&path, &ParserSettings::default());
        std::fs::remove_file(&path).expect("cleanup");
        assert!(matches!(result, Err(EvtcError::Zip(_))));
    }
}
