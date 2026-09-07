//! 错误类型。`Display` 文本逐字对齐 C# 异常消息（`GW2EIEvtcParser/Exceptions/`），
//! 便于后续黄金对拍阶段做行为等价验证。
//!
//! C# 侧所有解析失败都收敛到 `ParsingFailureReason` 返回 `null`；Rust 侧全部走
//! 显式 `Result<_, EvtcError>`，不保留任何静默 fallback。C# 的「静默容错」
//! （revision 非 0 → rev1、尾数忽略、ArcBuild 解析失败回退 header）在
//! `log_reader.rs` 复刻行为并逐一标注 C# 位置。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvtcError {
    /// C# `EvtcFileException("Not EVTC")`（EvtcParser.cs:54, 490-493）：
    /// 文件扩展名不支持、header magic 非 `EVTC`、或版本部分不可 parse 为整数。
    #[error("Not EVTC")]
    NotEvtc,
    /// C# `EvtcFileException("Invalid Archive")`（EvtcParser.cs:61-64）：
    /// 压缩包里 entry 数 != 1。
    #[error("Invalid Archive")]
    InvalidArchive,
    /// C# `TooBigException`（EvtcParser.cs:70-73）：`File is too big: {size} mb > {limit} mb`。
    #[error("File is too big: {size} mb > {limit} mb")]
    TooBig { size: u64, limit: u64 },
    /// C# `TooShortException`（EvtcParser.cs:904-907）：`Log is too short: {duration} ms < {limit} ms`。
    #[error("Log is too short: {duration} ms < {limit} ms")]
    TooShort { duration: i64, limit: i64 },
    /// C# `TooLongException`（EvtcParser.cs:909-912）：日志超过 24 小时。
    #[error("Log is longer than 24h")]
    TooLong,
    /// C# `EvtcCombatEventException("No combat events found")`（EvtcParser.cs:900-903）。
    #[error("No combat events found")]
    NoCombatEvents,
    /// 读取越过缓冲区结尾（C# `BinaryReader` 抛 `EndOfStreamException`）。
    /// C# 的 zip/压缩路径对截断日志无专门处理，Rust 显式报错。
    #[error(
        "Unexpected end of EVTC data at byte {position}. Required {required} bytes, remaining {remaining}."
    )]
    UnexpectedEnd {
        position: usize,
        required: usize,
        remaining: usize,
    },
    /// C# `EvtcFileException("File {FullName} does not exist")`（EvtcParser.cs:48-51）。
    #[error("File {path} does not exist")]
    FileNotFound { path: String },
    /// `std::io` 错误（文件读取 / zip 解压 IO）。
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// `zip` crate 错误（.NET `ZipArchive`/`InvalidDataException` 的对应物）。
    #[error("Zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
}
