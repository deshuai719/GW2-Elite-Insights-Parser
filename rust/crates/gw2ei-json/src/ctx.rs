//! 构建上下文与顶层装配:BuildError/BuildOptions + 日志标量元数据
//! (LogMetadata.cs 面)+ `build_report` 主入口。
//!
//! WvW 路径(trigger == 1;其他 encounter 由 gw2ei-model 拒绝)。

use std::collections::BTreeMap;
use std::path::Path;

use gw2ei_model::events::CombatEvent;
use gw2ei_model::ParsedLog;
use gw2ei_parse::read_evtc_log;

use crate::content::{self, BuffRegistry, Content};
use crate::dto_a::JsonLog;
use crate::rows::RowIndex;
use crate::stats::DownMap;

// ===== 错误 =====
#[derive(Debug)]
pub enum BuildError {
    /// 资产缺失/解析失败。
    Content(String),
    /// 输入日志问题。
    Log(String),
    /// 输出序列化错误(内部)。
    Io(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Content(s) | BuildError::Log(s) | BuildError::Io(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<gw2ei_model::ModelError> for BuildError {
    fn from(e: gw2ei_model::ModelError) -> Self {
        BuildError::Log(e.to_string())
    }
}

// ===== 选项(解析设置如实写进 JSON parsingSettings;golden conf 全 true)=====
#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub parse_extensions: bool,
    pub compute_phases: bool,
    pub compute_combat_replay: bool,
    pub compute_damage_modifiers: bool,
    pub compute_damage: bool,
    pub compute_cast: bool,
    pub compute_buff: bool,
    pub compute_mechanics: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        // 与 rust/tools/ei-golden.conf 一致(dps.report 近似全量)
        BuildOptions {
            parse_extensions: true,
            compute_phases: true,
            compute_combat_replay: true,
            compute_damage_modifiers: true,
            compute_damage: true,
            compute_cast: true,
            compute_buff: true,
            compute_mechanics: true,
        }
    }
}

// ===== 时间格式化(.NET ToLocalTime + "yyyy-MM-dd HH:mm:ss zz|zzz")=====

pub fn format_unix_seconds(secs: f64, std: bool) -> String {
    let secs_i = secs.floor() as i64;
    // chrono::Local 的偏移在 +08 机器上稳定;跨时区比对会不同 —— 与 EI CLI
    // 同机生成 golden,约束一致。
    let dt = chrono::DateTime::from_timestamp(secs_i, 0)
        .expect("unix seconds in range")
        .with_timezone(&chrono::Local);
    let off = dt.offset().local_minus_utc();
    let sign = if off < 0 { '-' } else { '+' };
    let off = off.unsigned_abs();
    let (h, m) = (off / 3600, (off % 3600) / 60);
    let base = dt.format("%Y-%m-%d %H:%M:%S").to_string();
    if std {
        format!("{base} {sign}{h:02}:{m:02}")
    } else {
        format!("{base} {sign}{h:02}")
    }
}

/// C# ParserHelper.ToDurationString(ms):mm 两位分 + ss 两位秒 + 余毫秒。
pub fn duration_string(duration: i64) -> String {
    let neg = duration < 0;
    let abs = duration.abs();
    let (mm, ss, ms) = (abs / 60_000, (abs % 60_000) / 1000, abs % 1000);
    let mut s = format!("{mm:02}m {ss:02}s {ms}ms");
    if abs >= 3_600_000 {
        s = format!("{:02}h {s}", abs / 3_600_000);
    }
    if neg {
        s = format!("-{s}");
    }
    s
}

// ===== 日志标量(LogMetadata.cs 最小面)=====

/// 从事件流提取的元数据(一次扫描)。
pub struct MetaSig {
    pub gw2_build: u64,
    pub language: Option<gw2ei_parse::Language>,
    pub region: Option<meta::RegionLabel>,
    pub pov_addr: Option<u64>,
    pub pov_time: Option<i64>,
    pub unix_start: Option<u32>,
    pub unix_end: Option<u32>,
    pub instance_time_offset: Option<i64>,
    pub instance_ip: Option<String>,
    pub errors: Vec<String>,
    pub guild_events: Vec<(u64, String)>,
    pub fractal_scale: Option<u8>,
    pub map_id_events: Vec<i64>,
    /// TeamChange 行:按 agent 地址(最后一行 teamID)。
    pub team_changes: Vec<(u64, i64, i64, i64)>, // (src addr, time, into, from)
    pub wvw_teams: Option<meta::WvwTeamsSig>,
}

pub mod meta {
    use gw2ei_model::events::Region;

    pub enum RegionLabel {
        Na,
        Eu,
        Cn,
        Unknown,
    }
    impl RegionLabel {
        pub fn json(&self) -> String {
            match self {
                RegionLabel::Na => "North America".to_string(),
                RegionLabel::Eu => "Europe".to_string(),
                RegionLabel::Cn => "China".to_string(),
                RegionLabel::Unknown => "Unknown".to_string(),
            }
        }
    }
    impl From<Region> for RegionLabel {
        fn from(r: Region) -> Self {
            match r {
                Region::Na => RegionLabel::Na,
                Region::Eu => RegionLabel::Eu,
                Region::Cn => RegionLabel::Cn,
                Region::Unknown => RegionLabel::Unknown,
            }
        }
    }
    pub struct WvwTeamsSig {
        pub red_shard: u32,
        pub blue_shard: u32,
        pub green_shard: u32,
        pub red_team: u32,
        pub blue_team: u32,
        pub green_team: u32,
    }
}

pub fn collect_meta(log: &ParsedLog) -> MetaSig {
    let md = &log.metadata;
    let mut m = MetaSig {
        gw2_build: md.gw2_build.as_ref().map(|g| g.build).unwrap_or(0),
        language: md.language.as_ref().map(|l| l.language),
        region: md.shard.as_ref().map(|sh| sh.region.into()),
        pov_addr: md.point_of_view.as_ref().map(|p| p.pov),
        pov_time: md.point_of_view.as_ref().map(|p| p.time),
        unix_start: md
            .log_start_event
            .as_ref()
            .map(|e| e.date.server_unix_time_stamp),
        unix_end: md
            .log_end_event
            .as_ref()
            .map(|e| e.date.server_unix_time_stamp),
        instance_time_offset: md.instance_start.as_ref().map(|e| e.time_offset_from_instance_creation),
        instance_ip: md.instance_start.as_ref().and_then(|e| {
            e.instance_ip
                .map(|ip| format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]))
        }),
        errors: md.errors.iter().map(|e| e.message.clone()).collect(),
        guild_events: md
            .guilds
            .iter()
            .map(|g| (g.src, g.guild_key_hex.clone()))
            .collect(),
        fractal_scale: md.fractal_scale.as_ref().map(|f| f.scale),
        map_id_events: {
            let mut v = Vec::new();
            if let Some(m) = &md.map_id {
                v.push(i64::from(m.map_id));
            }
            v.extend(md.map_changes.iter().map(|c| i64::from(c.map_id)));
            v
        },
        team_changes: Vec::new(),
        wvw_teams: md.wvw_teams.as_ref().map(|w| meta::WvwTeamsSig {
            red_shard: w.red_shard_id,
            blue_shard: w.blue_shard_id,
            green_shard: w.green_shard_id,
            red_team: w.red_team_id,
            blue_team: w.blue_team_id,
            green_team: w.green_team_id,
        }),
    };
    // TeamChange(F 组行级,在 events)
    for evt in &log.events {
        if let CombatEvent::TeamChange(f) = evt {
            m.team_changes.push((
                f.base.src,
                f.base.time,
                f.team_id_into as i64,
                f.team_id_coming_from as i64,
            ));
        }
    }
    m
}

/// 日志全程构建上下文。
pub struct Ctx<'a> {
    pub log: &'a ParsedLog,
    pub content: &'a Content,
    pub opts: &'a BuildOptions,
    pub meta: MetaSig,
    pub registry: BuffRegistry,
    pub rows: RowIndex,
    pub aux: crate::rows::AuxIndex,
    pub idx: gw2ei_model::EventIndex,
    /// buff id → (category byte, max stacks, 存在)
    pub buff_info: BTreeMap<u32, (u8, u16, bool)>,
    /// 承伤者 down/health 状态表(每行 `to` 判定 DownContribution)。
    pub downs: DownMap,
    pub map_id: i64,
    pub log_start: i64,
    pub log_end: i64,
}

/// 主入口:日志(已组装 ParsedLog)+ 资产 → JsonLog。
/// `parse_and_build` 直接读文件。
pub fn build_report(log: &ParsedLog, content: &Content, opts: &BuildOptions) -> Result<JsonLog, BuildError> {
    let meta = collect_meta(log);
    let log_start = log.log_data.log_start;
    let log_end = log.log_data.log_end;
    // BuffInfo 事件(P1 在 combat_data.buff_info_by_id 合并;BuffsContainer
    // 动态食物/强化合成依赖 category 字节)。
    let buff_info: BTreeMap<u32, (u8, u16, bool)> = log
        .metadata
        .buff_info_by_id
        .iter()
        .filter_map(|(&id, evt)| {
            evt.info
                .as_ref()
                .map(|i| (id, (i.category_byte, i.max_stacks, i.duration_cap > 0)))
        })
        .collect();
    let registry = content::registry_for(
        content,
        meta.gw2_build,
        i64::from(log.arc_version.0),
        &buff_info,
    );
    let rows = RowIndex::new(log);
    let aux = crate::rows::AuxIndex::new(log);
    let idx = gw2ei_model::build_event_index(log);
    let downs = DownMap::new(log, &idx);
    let map_id = meta.map_id_events.first().copied().unwrap_or(0);
    let ctx = Ctx {
        log,
        content,
        opts,
        meta,
        registry,
        rows,
        aux,
        idx,
        buff_info,
        downs,
        map_id,
        log_start,
        log_end,
    };
    crate::build::build_json(&ctx)
}

/// 完整管线:读文件 → 组装(带 SpecList)→ 资产 → JsonLog。
pub fn parse_and_build(
    log_path: &Path,
    content_dir: &Path,
    opts: &BuildOptions,
) -> Result<JsonLog, BuildError> {
    let content = content::load_default_content(content_dir)?;
    let raw = read_evtc_log(log_path).map_err(|e| BuildError::Log(e.to_string()))?;
    let raw2 = raw;
    let log = gw2ei_model::assemble_raw_with(
        raw2,
        &content::CatalogSpecs(&content),
    )?;
    build_report(&log, &content, opts)
}
