//! EVTC 二进制解析主体：`ParseLogData → ParseAgentData → ParseSkillData →
//! ParseCombatList`（EvtcParser.cs:485-914）的 Rust 实现。
//!
//! 与 C# 对齐的「静默容错」点（刻意复刻而非报错，均已注释标注 C# 位置）：
//! - revision 非 0 一律按 rev1 读（EvtcParser.cs:825，无取值校验）；
//! - combat 区尾数（不足 64B）静默忽略（EvtcParser.cs:809 `(Length-Position)/64`）；
//! - ArcBuild 事件 48B payload 解析失败回退 header 值、Revision=0（EvtcVersionEvent.cs:15-70）。
//!
//! 有意偏差（相对 C#，均为显式错误而非 C# 的 catch-all 返回 null）：
//! - 读取越界（截断文件）→ `EvtcError::UnexpectedEnd`（C# 在 `ParseLog` 外层
//!   catch 后以 `ParsingFailureReason` 吞掉，Rust 显式暴露）。

use std::path::Path;

use crate::enums::{Iff, LogType, StateChange};
use crate::error::EvtcError;
use crate::models::{
    DiscardStats, EvtcCombatItem, EvtcRawAgent, EvtcRawLog, EvtcRawSkill, ParseStats,
    ParserSettings,
};
use crate::reader::Reader;
use crate::zip_reader;

/// 按扩展名分派读取并解析日志文件。
/// - `.evtc` → 直接读；`.zevtc` / `.evtc.zip` → 解压（恰 1 entry）后解析。
/// - 其余扩展名 → `EvtcError::NotEvtc`（对齐 `EvtcFileException("Not EVTC")`，
///   EvtcParser.cs:52-55）。
pub fn read_evtc_log<P: AsRef<Path>>(path: P) -> Result<EvtcRawLog, EvtcError> {
    let path = path.as_ref();
    let lower = path.to_string_lossy().to_lowercase();
    let (is_plain, is_compressed) = if lower.ends_with(".evtc") {
        (true, false)
    } else if lower.ends_with(".zevtc") || lower.ends_with(".evtc.zip") {
        (false, true)
    } else {
        return Err(EvtcError::NotEvtc);
    };
    let settings = ParserSettings::default();
    let bytes = if is_compressed {
        zip_reader::read_compressed_log(path, &settings)?
    } else if is_plain {
        std::fs::read(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => EvtcError::FileNotFound {
                path: path.display().to_string(),
            },
            _ => EvtcError::Io(e),
        })?
    } else {
        unreachable!()
    };
    parse_evtc_log_with_settings(&bytes, &settings)
}

/// 解析内存中的 EVTC 字节流（默认 `ParserSettings`）。
pub fn parse_evtc_log(bytes: &[u8]) -> Result<EvtcRawLog, EvtcError> {
    parse_evtc_log_with_settings(bytes, &ParserSettings::default())
}

/// 解析内存中的 EVTC 字节流（自定义设置）。
pub fn parse_evtc_log_with_settings(
    bytes: &[u8],
    settings: &ParserSettings,
) -> Result<EvtcRawLog, EvtcError> {
    let mut reader = Reader::new(bytes);

    // ===== ParseLogData（EvtcParser.cs:485-507）=====
    let version12 = reader.take_bytes(12)?;
    let header_build = parse_header_build(version12)?;
    let revision = reader.read_u8()?;
    let id = reader.read_u16()?;
    // 偏移 15 的 1B 丢弃（header 尾）。
    reader.skip(1)?;

    // ===== ParseAgentData（EvtcParser.cs:533-590）=====
    let agent_count = reader.read_u32()?;
    let mut agents = Vec::with_capacity(agent_count as usize);
    for _ in 0..agent_count {
        agents.push(read_agent(&mut reader)?);
    }

    // ===== ParseSkillData（EvtcParser.cs:599-618）=====
    let skill_count = reader.read_u32()?;
    let mut skills = Vec::with_capacity(skill_count as usize);
    for _ in 0..skill_count {
        skills.push(read_skill(&mut reader)?);
    }

    // ===== ParseCombatList（EvtcParser.cs:805-914）=====
    // 64 bytes each；count = (Length - Position) / 64，尾数静默忽略（:809）。
    let raw_combat_count = (reader.remaining() / 64) as u64;

    // stopAtLogEndEvent：instance 日志(id==2) 1，boss 日志 -1（:813）。
    let mut effective_id: u16 = id;
    let mut stop_at_log_end_event: i32 = if id == 2 { 1 } else { -1 };
    let mut keep_only_extension_events = false;
    let mut current_map_id: i32 = -1;
    let mut map_id: i32 = -1;
    let mut log_start_offset: i64 = i64::MIN;
    let mut log_end_time: i64 = 0;
    let mut gw2_build: u64 = 0;
    let mut arc_version_build = header_build;
    let mut arc_version_revision = -1; // C# EvtcVersionEvent.Revision 初值
    let mut redirection = std::collections::BTreeMap::new();
    let mut enabled_extensions: Vec<u32> = Vec::new();
    let mut arc_build_consumed: u64 = 0;
    let mut discarded = DiscardStats::default();

    let mut combat_events: Vec<EvtcCombatItem> =
        Vec::with_capacity(raw_combat_count as usize);
    let mut extension_indices: Vec<usize> = Vec::new();

    for _ in 0..raw_combat_count {
        let mut item = if revision > 0 {
            read_combat_item_rev1(&mut reader)?
        } else {
            read_combat_item_rev0(&mut reader)?
        };
        // SquadCombatStart 的日志类型修正（:826-840）：boss 日志里出现
        // Map 型 SquadCombatStart → 日志实际是 instance 日志。
        if stop_at_log_end_event == -1
            && item.state_change == StateChange::SquadCombatStart
        {
            if item.squad_combat_log_type() == LogType::Map {
                effective_id = 2;
                stop_at_log_end_event = 1;
            } else {
                stop_at_log_end_event = 0;
            }
        }
        // MapID / MapChange 跟踪 currentMapID（:841-849）。
        if item.state_change == StateChange::MapID {
            map_id = item.src_agent as i32;
            current_map_id = map_id;
        }
        if item.state_change == StateChange::MapChange {
            current_map_id = item.src_agent as i32;
        }
        // AgentChange 重定向表收集，不进流（:850-853）。
        if item.state_change == StateChange::AgentChange {
            redirection.insert(item.src_agent, item.dst_agent);
        }

        // IsValid 6 条过滤 + keepOnlyExtensionEvents 裁尾（:854-857）。
        // 计数只归一类：IsValid 失败记具体原因；仅被裁尾才记 after_log_end。
        let invalid = !is_valid(
            &item,
            settings,
            current_map_id,
            &mut enabled_extensions,
            effective_id,
            map_id,
            &mut discarded,
        );
        let trimmed_by_log_end = keep_only_extension_events && !item.is_extension();
        if invalid || trimmed_by_log_end {
            if !invalid && trimmed_by_log_end {
                discarded.after_log_end += 1;
            }
            continue;
        }

        // ArcBuild：SetFromCombatItem 后 continue，不进列表（:860-864）。
        //（不计数为丢弃 —— 与 C# 一致；守恒核对见 arc_build_consumed。）
        if item.state_change == StateChange::ArcBuild {
            arc_build_consumed += 1;
            match parse_arc_build_payload(&item) {
                Some((build, revision)) => {
                    arc_version_build = build;
                    arc_version_revision = revision;
                }
                // aligned with C#：解析失败回退 header 值、Revision=0
                // （EvtcVersionEvent.cs:15-70 的 catch 分支）。
                None => {
                    arc_version_build = header_build;
                    arc_version_revision = 0;
                }
            }
            continue;
        }

        // 时间归零（:866-874）：HasTime 白名单事件参与。
        if item.has_time() {
            if log_start_offset == i64::MIN {
                log_start_offset = item.time;
            }
            // C# 直接相减（unchecked）；首事件修正后 logEndTime 即归零末事件时间。
            item.time = item.time.wrapping_sub(log_start_offset);
            log_end_time = item.time;
        }

        combat_events.push(item);
        if item.is_extension() {
            extension_indices.push(combat_events.len() - 1);
        }

        // GWBuild 非 0 时记录（:882-885）。
        if item.state_change == StateChange::GWBuild && item.src_agent != 0 {
            gw2_build = item.src_agent;
        }
        // SquadCombatEnd 且 stopAtLogEndEvent <= 0 → 后续仅保留 extension 行
        // （:887-890，boss 日志在末次战斗结束时裁尾）。
        if item.state_change == StateChange::SquadCombatEnd && stop_at_log_end_event <= 0 {
            keep_only_extension_events = true;
        }
    }

    // extension 行二次时间归零（:892-898）。循环内 HasTime() 对 extension 行
    // 恒 false（不在白名单），此处按 HasTime(enabledExtensions) 修正 ——
    // 已注册扩展的 handler 声明 HasTime=true（HealingStatsExtensionHandler.cs:207）。
    // aligned with C#：无条件减 logStartOffset（unchecked），无溢出保护。
    for &idx in &extension_indices {
        let item = &mut combat_events[idx];
        if item.is_extension()
            && item.pad != 0
            && enabled_extensions.contains(&item.pad)
        {
            item.time = item.time.wrapping_sub(log_start_offset);
        }
    }

    // 数量/时长检查（:900-912）。
    if combat_events.is_empty() {
        return Err(EvtcError::NoCombatEvents);
    }
    if log_end_time < settings.too_short_limit_ms {
        return Err(EvtcError::TooShort {
            duration: log_end_time,
            limit: settings.too_short_limit_ms,
        });
    }
    // 24 小时上限。
    if log_end_time > 86_400_000 {
        return Err(EvtcError::TooLong);
    }

    let kept_combat_count = combat_events.len();
    let stats = ParseStats {
        raw_agent_count: agent_count,
        raw_skill_count: skill_count,
        raw_combat_count,
        kept_combat_count,
        arc_build_consumed,
        discarded,
    };

    Ok(EvtcRawLog {
        header_build,
        revision,
        id: effective_id,
        agents,
        skills,
        combat_events,
        agent_redirection: redirection,
        enabled_extensions,
        map_id,
        log_start_offset,
        log_end_time,
        arc_version: (arc_version_build, arc_version_revision),
        gw2_build,
        stats,
    })
}

/// ParseLogData 的 12B 版本串校验：`StartsWith("EVTC")` 且 `[4..]` 可
/// `int.TryParse`（EvtcParser.cs:490-493）。C# 把字节逐位 cast 成 char 再做
/// TryParse，仅 ASCII 数字（允许首尾空白与 +/- 号）可通过；此处置为逐字节解析，
/// 任一字节非法/溢出 → "Not EVTC"。
fn parse_header_build(version12: &[u8]) -> Result<i32, EvtcError> {
    if !version12.starts_with(b"EVTC") {
        return Err(EvtcError::NotEvtc);
    }
    let digits = version12[4..].to_vec();
    let s = trim_ascii_whitespace(&digits);
    let mut negative = false;
    let mut rest = s;
    if let Some((&first, tail)) = rest.split_first() {
        match first {
            b'+' => rest = tail,
            b'-' => {
                negative = true;
                rest = tail;
            }
            _ => {}
        }
    }
    let mut value: i64 = 0;
    for &b in rest {
        if !b.is_ascii_digit() {
            return Err(EvtcError::NotEvtc);
        }
        value = value * 10 + (b - b'0') as i64;
        if value > (i32::MAX as i64) + 1 {
            return Err(EvtcError::NotEvtc); // int.TryParse 溢出即失败
        }
    }
    if rest.is_empty() {
        return Err(EvtcError::NotEvtc);
    }
    let value = if negative { -value } else { value };
    Ok(value as i32)
}

fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let is_ws = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let start = bytes.iter().position(|b| !is_ws(*b)).unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !is_ws(*b))
        .map_or(start, |i| i + 1);
    &bytes[start..end]
}

/// 读取 96B agent 记录（EvtcParser.cs:544-587 的读取序）。类型/名字语义化在
/// `EvtcRawAgent::agent_kind` / P2 `AgentItem`。
fn read_agent(reader: &mut Reader<'_>) -> Result<EvtcRawAgent, EvtcError> {
    let address = reader.read_u64()?;
    let prof = reader.read_u32()?;
    let is_elite = reader.read_u32()?;
    let toughness = reader.read_u16()?;
    let concentration = reader.read_u16()?;
    let healing = reader.read_u16()?;
    // 第 4/5/6 个 u16 的读取序：hitbox_width / condition / hitbox_height
    // （C# 同序，EvtcParser.cs:559-564）。
    let hitbox_width_raw = reader.read_u16()?;
    let condition = reader.read_u16()?;
    let hitbox_height_raw = reader.read_u16()?;
    // 68B 名字原样保留（GetString(reader, 68, nullTerminated: false)，:566）。
    let name_bytes = reader.take_bytes(68)?;
    let mut name = [0u8; 68];
    name.copy_from_slice(name_bytes);
    Ok(EvtcRawAgent {
        address,
        prof,
        is_elite,
        toughness,
        concentration,
        healing,
        hitbox_width_raw,
        condition,
        hitbox_height_raw,
        name,
    })
}

/// 读取 68B skill 记录（EvtcParser.cs:599-618）。
/// - id 按 `ReadInt32` 读（i32 位型存 u32）。
/// - 名字 64B 按首个 `\0` 截断（GetString(reader, 64) 默认 nullTerminated=true）。
fn read_skill(reader: &mut Reader<'_>) -> Result<EvtcRawSkill, EvtcError> {
    let id_raw = reader.read_i32()?;
    let name_bytes = reader.take_bytes(64)?;
    let nul = name_bytes.iter().position(|&b| b == 0).unwrap_or(64);
    let name = String::from_utf8_lossy(&name_bytes[..nul]).into_owned();
    Ok(EvtcRawSkill {
        id: id_raw as u32,
        name,
    })
}

/// rev0 布局（EvtcParser.cs:627-706）：
/// overstack/skill_id 为 u16；无 dst_master_instid —— 42-50 共 9 字节整段
/// 跳过（dstMaster 恒 0）；标志字节偏移整体前移（iff@51 … is_offcycle@62）；
/// 偏移 63 有 1B 垃圾跳过；pad 恒 0（rev0 无扩展通道）。
fn read_combat_item_rev0(reader: &mut Reader<'_>) -> Result<EvtcCombatItem, EvtcError> {
    let time = reader.read_i64()?;
    let src_agent = reader.read_u64()?;
    let dst_agent = reader.read_u64()?;
    let value = reader.read_i32()?;
    let buff_dmg = reader.read_i32()?;
    let overstack_value = reader.read_u16()? as u32;
    let skill_id = reader.read_u16()? as u32;
    let src_instid = reader.read_u16()?;
    let dst_instid = reader.read_u16()?;
    let src_master_instid = reader.read_u16()?;
    // 9 bytes: garbage（C# 拆成 ReadInt64 + ReadByte 消费）。
    reader.skip(9)?;
    let iff_raw = reader.read_u8()?;
    let buff = reader.read_u8()?;
    let result = reader.read_u8()?;
    let activation_raw = reader.read_u8()?;
    let buff_remove_raw = reader.read_u8()?;
    let is_ninety = reader.read_u8()?;
    let is_fifty = reader.read_u8()?;
    let is_moving = reader.read_u8()?;
    let state_change_raw = reader.read_u8()?;
    let is_flanking = reader.read_u8()?;
    let is_shields = reader.read_u8()?;
    let is_offcycle = reader.read_u8()?;
    // 1 byte: garbage（:699）。
    reader.skip(1)?;
    Ok(EvtcCombatItem {
        time,
        src_agent,
        dst_agent,
        value,
        buff_dmg,
        overstack_value,
        skill_id,
        src_instid,
        dst_instid,
        src_master_instid,
        dst_master_instid: 0,
        iff: Iff::from_byte(iff_raw),
        iff_raw,
        buff,
        result,
        activation: crate::enums::Activation::from_byte(activation_raw),
        activation_raw,
        buff_remove: crate::enums::BuffRemove::from_byte(buff_remove_raw),
        buff_remove_raw,
        is_ninety,
        is_fifty,
        is_moving,
        state_change: StateChange::from_byte(state_change_raw),
        state_change_raw,
        is_flanking,
        is_shields,
        is_offcycle,
        pad: 0,
    })
}

/// rev1 布局（EvtcParser.cs:715-791）：全字段 + dst_master_instid + 4B pad。
fn read_combat_item_rev1(reader: &mut Reader<'_>) -> Result<EvtcCombatItem, EvtcError> {
    let time = reader.read_i64()?;
    let src_agent = reader.read_u64()?;
    let dst_agent = reader.read_u64()?;
    let value = reader.read_i32()?;
    let buff_dmg = reader.read_i32()?;
    let overstack_value = reader.read_u32()?;
    let skill_id = reader.read_u32()?;
    let src_instid = reader.read_u16()?;
    let dst_instid = reader.read_u16()?;
    let src_master_instid = reader.read_u16()?;
    let dst_master_instid = reader.read_u16()?;
    let iff_raw = reader.read_u8()?;
    let buff = reader.read_u8()?;
    let result = reader.read_u8()?;
    let activation_raw = reader.read_u8()?;
    let buff_remove_raw = reader.read_u8()?;
    let is_ninety = reader.read_u8()?;
    let is_fifty = reader.read_u8()?;
    let is_moving = reader.read_u8()?;
    let state_change_raw = reader.read_u8()?;
    let is_flanking = reader.read_u8()?;
    let is_shields = reader.read_u8()?;
    let is_offcycle = reader.read_u8()?;
    let pad = reader.read_u32()?;
    Ok(EvtcCombatItem {
        time,
        src_agent,
        dst_agent,
        value,
        buff_dmg,
        overstack_value,
        skill_id,
        src_instid,
        dst_instid,
        src_master_instid,
        dst_master_instid,
        iff: Iff::from_byte(iff_raw),
        iff_raw,
        buff,
        result,
        activation: crate::enums::Activation::from_byte(activation_raw),
        activation_raw,
        buff_remove: crate::enums::BuffRemove::from_byte(buff_remove_raw),
        buff_remove_raw,
        is_ninety,
        is_fifty,
        is_moving,
        state_change: StateChange::from_byte(state_change_raw),
        state_change_raw,
        is_flanking,
        is_shields,
        is_offcycle,
        pad,
    })
}

/// `IsValid` 六条过滤（EvtcParser.cs:926-976）。返回 false 的行被丢弃并分类计数。
///
/// 偏差说明：C# 的 ② 与空事件⑥ 判定中 `SrcIsAgent(enabledExtensions)`/
/// `DstIsAgent(enabledExtensions)` 会把已注册扩展行的判定委托给 handler
/// （HealingStats 扩展声明其事件涉 agent）；P1 不做扩展行内容解析，扩展行
/// 按无扩展白名单（恒 false）处理 —— 影响的仅是「地图切换边界上的扩展行」
/// 这类边缘行是否丢弃，healing/barrier 语义在 P3 引入时再对齐。
fn is_valid(
    item: &EvtcCombatItem,
    settings: &ParserSettings,
    current_map_id: i32,
    enabled_extensions: &mut Vec<u32>,
    id: u16,
    map_id: i32,
    discarded: &mut DiscardStats,
) -> bool {
    // ① 受支持的 state change（ParserHelper.cs:196-203：排除 Unknown/ReplInfo/
    // StatReset/APIDelayed/Idle/AgentChange/EarlyExit/Jump）。
    if matches!(
        item.state_change,
        StateChange::Unknown
            | StateChange::ReplInfo
            | StateChange::StatReset
            | StateChange::APIDelayed
            | StateChange::Idle
            | StateChange::AgentChange
            | StateChange::EarlyExit
            | StateChange::Jump
    ) {
        discarded.unsupported_state_change += 1;
        return false;
    }
    // ② 事件涉 agent 但地图已切换（mapID != currentMapID）。
    if map_id != -1
        && map_id != current_map_id
        && (item.src_is_agent() || item.dst_is_agent())
    {
        discarded.map_mismatch += 1;
        return false;
    }
    // ③ instance 日志（id == TargetID.Instance == 2）丢弃 BuffInitial
    //（ParserHelper.cs:204-207）。
    if id == 2 && item.state_change == StateChange::BuffInitial {
        discarded.instance_buff_initial += 1;
        return false;
    }
    // ④ HealthUpdate 行血量 > 200% 丢弃。C# 比较的是
    // `Math.Round(DstAgent/100.0, 2) > 200`（double 语义，HealthUpdateEvent.cs:18-20）
    // ⇔ `DstAgent > 20000`；不能用整数除法（dst ∈ 20001..=20099 会误判不丢）。
    if item.state_change == StateChange::HealthUpdate && item.dst_agent > 20_000 {
        discarded.invalid_health_update += 1;
        return false;
    }
    // ⑤ extension 行（:947-970）。
    if item.is_extension() {
        if !settings.parse_extensions {
            discarded.extensions_disabled += 1;
            return false;
        }
        // 首个元事件（Pad==0 且 StateChange.Extension）：尝试注册 handler 后
        // 自身总是丢弃。
        if item.pad == 0 && item.state_change == StateChange::Extension {
            let (sig, rev) = crate::models::parse_extension_header(item.src_agent);
            if crate::models::extension_supported(sig, rev) {
                // 注册签名（C# enabledExtensions[handler.Signature] = handler）。
                enabled_extensions.push(sig);
            }
            // No need to keep that event（:963-964）—— 无论 handler 是否支持。
            discarded.extension_metadata += 1;
            return false;
        }
        if !enabled_extensions.contains(&item.pad) {
            discarded.unknown_extension_pad += 1;
            return false;
        }
    }
    // ⑥ 空事件：instid/agent 全 0 且 IFF Unknown 且非 effect/missile（:971-974）。
    if item.src_instid == 0
        && item.dst_agent == 0
        && item.src_agent == 0
        && item.dst_instid == 0
        && item.iff == Iff::Unknown
        && !item.is_effect()
        && !item.is_missile()
    {
        discarded.empty_event += 1;
        return false;
    }
    true
}

/// ArcBuild 事件 payload 解析（EvtcVersionEvent.cs:28-66 `SetFromCombatItem`）。
///
/// 48B = SrcAgent(8) DstAgent(8) Value(4) BuffDmg(4) Overstack(4) SkillID(4)
/// SrcInstid(2) DstInstid(2) SrcMaster(2) DstMaster(2) IFF(1) Buff(1) Result(1)
/// Activation(1) BuffRemove(1) IsNinety(1) IsFifty(1) IsMoving(1)，按序回写小端
/// 后按 UTF-8 解码（TrimEnd('\0')）。文本形如 `"{build}.{revision}"`：
/// Build = 首个 `.` 左侧，Revision = 首个 `.` 右侧再按 `-` 分节后的左侧
/// （`StringExt.SplitOnce` 的 Tail 语义 = 分隔符左段，StringExt.cs:21-28）。
/// 任一步解析失败 → 回退（None，调用方回退 header 值、Revision=0）。
fn parse_arc_build_payload(item: &EvtcCombatItem) -> Option<(i32, i32)> {
    let mut buf = [0u8; 48];
    let mut off = 0;
    for chunk in [
        item.src_agent.to_le_bytes().as_slice(),
        item.dst_agent.to_le_bytes().as_slice(),
        item.value.to_le_bytes().as_slice(),
        item.buff_dmg.to_le_bytes().as_slice(),
        item.overstack_value.to_le_bytes().as_slice(),
        item.skill_id.to_le_bytes().as_slice(),
        item.src_instid.to_le_bytes().as_slice(),
        item.dst_instid.to_le_bytes().as_slice(),
        item.src_master_instid.to_le_bytes().as_slice(),
        item.dst_master_instid.to_le_bytes().as_slice(),
        &[item.iff_raw],
        &[item.buff],
        &[item.result],
        &[item.activation_raw],
        &[item.buff_remove_raw],
        &[item.is_ninety],
        &[item.is_fifty],
        &[item.is_moving],
    ] {
        buf[off..off + chunk.len()].copy_from_slice(chunk);
        off += chunk.len();
    }
    debug_assert_eq!(off, 48);
    // C# Encoding.UTF8.GetString：非法字节替换 U+FFFD（后续 int.Parse 失败
    // → 回退，与 C# replacement 后 TryParse 失败同路径）。
    let text = String::from_utf8_lossy(&buf);
    let text = text.trim_end_matches('\0');
    let build = parse_int_left_of(text, '.')?;
    let after_dot = text.split_once('.').map(|(_, r)| r).unwrap_or("");
    let revision = parse_int_left_of(after_dot, '-')?;
    Some((build, revision))
}

/// C# `int.Parse(SplitOnce(str, split).Tail)`：Tail = 分隔符左段；
/// 无分隔符时整串为左段（空串 parse 失败 → None）。
fn parse_int_left_of(text: &str, split: char) -> Option<i32> {
    let left = text.split(split).next().unwrap_or(text);
    let left = left.trim();
    left.parse::<i32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(revision: u8) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"EVTC20260507");
        v.push(revision);
        v.extend_from_slice(&2u16.to_le_bytes());
        v.push(0);
        v
    }

    fn agent_entry(name: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&0x1122334455667788u64.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes()); // prof
        v.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes()); // elite
        v.extend_from_slice(&1000u16.to_le_bytes()); // toughness
        v.extend_from_slice(&2000u16.to_le_bytes()); // concentration
        v.extend_from_slice(&3000u16.to_le_bytes()); // healing
        v.extend_from_slice(&4000u16.to_le_bytes()); // hitbox width
        v.extend_from_slice(&5000u16.to_le_bytes()); // condition
        v.extend_from_slice(&6000u16.to_le_bytes()); // hitbox height
        assert!(name.len() <= 68, "agent name must fit in 68 bytes");
        v.extend_from_slice(name);
        v.resize(96, 0); // 名字不足 68B 补 0
        v
    }

    fn skill_entry(id: u32, name: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&id.to_le_bytes());
        assert!(name.len() <= 64, "skill name must fit in 64 bytes");
        v.extend_from_slice(name);
        v.resize(68, 0); // 名字不足 64B 补 0
        v
    }

    /// rev1 combat 行构造（字段按 EvtcParser.cs:715-791 顺序）。
    #[allow(clippy::too_many_arguments)]
    fn combat_rev1(
        time: i64,
        src: u64,
        dst: u64,
        value: i32,
        buff_dmg: i32,
        overstack: u32,
        skill: u32,
        s_inst: u16,
        d_inst: u16,
        s_master: u16,
        d_master: u16,
        iff: u8,
        buff: u8,
        result: u8,
        activation: u8,
        buff_remove: u8,
        n90: u8,
        n50: u8,
        moving: u8,
        state: u8,
        flanking: u8,
        shields: u8,
        offcycle: u8,
        pad: u32,
    ) -> Vec<u8> {
        let mut v = Vec::new();
        for x in [
            time.to_le_bytes().as_slice(),
            src.to_le_bytes().as_slice(),
            dst.to_le_bytes().as_slice(),
            value.to_le_bytes().as_slice(),
            buff_dmg.to_le_bytes().as_slice(),
            overstack.to_le_bytes().as_slice(),
            skill.to_le_bytes().as_slice(),
            s_inst.to_le_bytes().as_slice(),
            d_inst.to_le_bytes().as_slice(),
            s_master.to_le_bytes().as_slice(),
            d_master.to_le_bytes().as_slice(),
            &[iff],
            &[buff],
            &[result],
            &[activation],
            &[buff_remove],
            &[n90],
            &[n50],
            &[moving],
            &[state],
            &[flanking],
            &[shields],
            &[offcycle],
            pad.to_le_bytes().as_slice(),
        ] {
            v.extend_from_slice(x);
        }
        assert_eq!(v.len(), 64);
        v
    }

    /// 一条有效的 combat 事件(rev1, Combat 行 + 有效 src 事件)。
    fn plain_combat_rev1(time: i64) -> Vec<u8> {
        combat_rev1(
            time, 0x1122334455667788, 0, 100, 0, 0, 7, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0,
        )
    }

    #[test]
    fn parses_valid_rev1_log() {
        let mut bytes = header(1);
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&agent_entry(b"NPC some foe"));
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&skill_entry(7, b"Fireball\0rest"));
        // 时间从 5000 起，多行验证归零。
        bytes.extend_from_slice(&plain_combat_rev1(500000));
        bytes.extend_from_slice(&plain_combat_rev1(504000));
        let log = parse_evtc_log(&bytes).expect("synthetic test bytes must parse");
        assert_eq!(log.header_build, 20260507);
        assert_eq!(log.revision, 1);
        assert_eq!(log.id, 2); // instance 触发 id
        assert_eq!(log.stats.raw_agent_count, 1);
        assert_eq!(log.stats.raw_skill_count, 1);
        assert_eq!(log.stats.raw_combat_count, 2);
        assert_eq!(log.stats.kept_combat_count, 2);
        assert_eq!(log.log_start_offset, 500000);
        assert_eq!(log.log_end_time, 4000); // 504000-500000
        assert_eq!(log.combat_events[0].time, 0);
        assert_eq!(log.combat_events[1].time, 4000);
        // agent 区原始保留
        assert_eq!(log.agents[0].address, 0x1122334455667788);
        assert_eq!(log.agents[0].name_cstr(), b"NPC some foe");
        // 名字原始 68B 不截断
        assert_eq!(log.agents[0].name.len(), 68);
        // skill 名字按 \0 截断
        assert_eq!(log.skills[0].name, "Fireball");
        assert_eq!(log.skills[0].id, 7);
    }

    #[test]
    fn rejects_bad_magic_and_bad_version() {
        // 坏 magic
        let mut bytes = header(1);
        bytes[0] = b'X';
        assert!(matches!(
            parse_evtc_log(&bytes),
            Err(EvtcError::NotEvtc)
        ));
        // 版本部分非整数
        let mut bytes = header(1);
        bytes[4..12].copy_from_slice(b"abcdefgh");
        assert!(matches!(
            parse_evtc_log(&bytes),
            Err(EvtcError::NotEvtc)
        ));
        // 截断到 10B
        let bytes = &header(1)[..10];
        assert!(matches!(
            parse_evtc_log(bytes),
            Err(EvtcError::UnexpectedEnd { .. })
        ));
    }

    #[test]
    fn revision_0_layout_with_garbage_skip() {
        // rev0：无 dst_master；42-50 共 9B 垃圾；63 偏移 1B 垃圾。
        let mut row = Vec::new();
        row.extend_from_slice(&300000i64.to_le_bytes());
        row.extend_from_slice(&0x11u64.to_le_bytes());
        row.extend_from_slice(&0x22u64.to_le_bytes());
        row.extend_from_slice(&5i32.to_le_bytes());
        row.extend_from_slice(&6i32.to_le_bytes());
        row.extend_from_slice(&0xABCDu16.to_le_bytes()); // overstack u16
        row.extend_from_slice(&7u16.to_le_bytes()); // skill_id u16
        row.extend_from_slice(&8u16.to_le_bytes()); // src_instid
        row.extend_from_slice(&9u16.to_le_bytes()); // dst_instid
        row.extend_from_slice(&10u16.to_le_bytes()); // src_master_instid
        row.extend_from_slice(&[0xAA; 9]); // 9B garbage
        row.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // iff=1 foe
        row.extend_from_slice(&[0xBB]); // garbage @63
        assert_eq!(row.len(), 64);

        let mut row2 = row.clone();
        row2[0..8].copy_from_slice(&600000i64.to_le_bytes()); // 第二条（时间差 >2200）
        let mut bytes = header(0);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&row);
        bytes.extend_from_slice(&row2);
        let log = parse_evtc_log(&bytes).expect("synthetic test bytes must parse");
        assert_eq!(log.stats.raw_combat_count, 2);
        let evt = &log.combat_events[0];
        assert_eq!(evt.time, 0);
        assert_eq!(evt.dst_master_instid, 0);
        assert_eq!(evt.pad, 0);
        assert_eq!(evt.overstack_value, 0xABCD);
        assert_eq!(evt.skill_id, 7);
        assert_eq!(evt.iff, Iff::Foe);
        // rev0 的 skill_id u16 上限（扩展回 u32 后原值）
        assert_eq!(evt.skill_id, 7);
    }

    #[test]
    fn revision_255_reads_as_rev1() {
        // aligned with C#：revision 非 0 一律按 rev1（EvtcParser.cs:825）。
        let mut bytes = header(255);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&combat_rev1(
            300000, 0x11, 0, 10, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ));
        bytes.extend_from_slice(&combat_rev1(
            305000, 0x11, 0, 20, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ));
        let log = parse_evtc_log(&bytes).expect("synthetic test bytes must parse");
        assert_eq!(log.revision, 255);
        assert_eq!(log.combat_events.len(), 2);
        assert_eq!(log.combat_events[0].value, 10);
        assert_eq!(log.combat_events[0].time, 0);
        assert_eq!(log.combat_events[1].time, 5000);
    }

    #[test]
    fn arc_build_payload_parsing() {
        // 48B payload 文本 "20260530.1"
        let mut row = combat_rev1(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            StateChange::ArcBuild as u8, 0, 0, 0, 0);
        // 重写字段区（从 src_agent 起）为文本
        let text = b"20260530.1";
        row[8..8 + text.len()].copy_from_slice(text);
        // 用原始字段访问等价构造：直接调 payload 解析函数
        let mut item = read_item_from(&row, 1);
        item.state_change = StateChange::from_byte(StateChange::ArcBuild as u8);
        let parsed = parse_arc_build_payload(&item).expect("synthetic test bytes must parse");
        assert_eq!(parsed, (20260530, 1));

        // 带 EVTC 前缀 → 解析失败 → 回退（None）
        let text = b"EVTC20260530.1";
        row[8..8 + text.len()].copy_from_slice(text);
        let mut item = read_item_from(&row, 1);
        item.state_change = StateChange::from_byte(StateChange::ArcBuild as u8);
        assert!(parse_arc_build_payload(&item).is_none());

        // 无点版本 → 失败
        let text = b"20260530";
        row[8..8 + text.len()].copy_from_slice(text);
        let mut item = read_item_from(&row, 1);
        item.state_change = StateChange::from_byte(StateChange::ArcBuild as u8);
        assert!(parse_arc_build_payload(&item).is_none());
    }

    fn read_item_from(bytes: &[u8], revision: u8) -> EvtcCombatItem {
        let mut r = Reader::new(bytes);
        if revision > 0 {
            read_combat_item_rev1(&mut r).expect("synthetic test bytes must parse")
        } else {
            read_combat_item_rev0(&mut r).expect("synthetic test bytes must parse")
        }
    }

    #[test]
    fn arc_build_event_sets_version_and_is_consumed() {
        let row = plain_combat_rev1(700000);
        let row2 = plain_combat_rev1(703000);
        // ArcBuild 行：48B payload 位于 src_agent..src_agent+48（rev1 字段区）。
        let mut arc_row = combat_rev1(
            3000, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            StateChange::ArcBuild as u8, 0, 0, 0, 0,
        );
        let text = b"20260602.2";
        arc_row[8..8 + text.len()].copy_from_slice(text);

        let mut bytes = header(1);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&arc_row);
        bytes.extend_from_slice(&row);
        bytes.extend_from_slice(&row2);
        let log = parse_evtc_log(&bytes).expect("synthetic test bytes must parse");
        assert_eq!(log.arc_version, (20260602, 2));
        // ArcBuild 行被消费，不进列表
        assert_eq!(log.combat_events.len(), 2);
        // 时间归零只对 HasTime 的普通行：logStartOffset 取第一个 HasTime 行
        assert_eq!(log.log_start_offset, 700000);
        assert_eq!(log.combat_events[0].time, 0);
        assert_eq!(log.log_end_time, 3000); // 703000-700000
    }

    #[test]
    fn too_short_rejected() {
        let mut bytes = header(1);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        // 一条 100ms 的 combat 行 → logEndTime=0 < 2200
        bytes.extend_from_slice(&plain_combat_rev1(100));
        let err = parse_evtc_log(&bytes).expect_err("synthetic bytes must fail");
        assert!(
            matches!(err, EvtcError::TooShort { .. }),
            "unexpected {err}"
        );
    }

    #[test]
    fn no_combat_events_rejected() {
        // 全被 IsValid ① 过滤（Jump 不受支持）
        let mut bytes = header(1);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&combat_rev1(
            1000, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            StateChange::Jump as u8, 0, 0, 0, 0,
        ));
        assert!(matches!(
            parse_evtc_log(&bytes),
            Err(EvtcError::NoCombatEvents)
        ));
    }

    #[test]
    fn empty_event_and_map_mismatch_filters() {
        // 空事件（全 0 + IFF Unknown）被⑥丢弃
        let empty = combat_rev1(1000, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0);
        let mut bytes = header(1);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&empty);
        bytes.extend_from_slice(&plain_combat_rev1(300000));
        bytes.extend_from_slice(&plain_combat_rev1(305000));
        let log = parse_evtc_log(&bytes).expect("synthetic test bytes must parse");
        assert_eq!(log.stats.kept_combat_count, 2);
        assert_eq!(log.stats.discarded.empty_event, 1);
        assert_eq!(log.stats.discarded.total(), 1);
    }

    #[test]
    fn health_update_over_200_percent_dropped() {
        // C# 判定 `Math.Round(DstAgent/100.0, 2) > 200` ⇔ DstAgent > 20000：
        // 20050（200.50%）丢弃、20000（200.00%）保留 —— 整数除法会误判前者的边界。
        let mut bad = combat_rev1(
            100000, 0x11, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
            StateChange::HealthUpdate as u8, 0, 0, 0, 0,
        );
        bad[16..24].copy_from_slice(&20050u64.to_le_bytes()); // dst_agent
        let mut good = combat_rev1(
            105000, 0x11, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
            StateChange::HealthUpdate as u8, 0, 0, 0, 0,
        );
        good[16..24].copy_from_slice(&20000u64.to_le_bytes()); // dst_agent
        let mut bytes = header(1);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&bad);
        bytes.extend_from_slice(&good);
        bytes.extend_from_slice(&plain_combat_rev1(500000));
        let log = parse_evtc_log(&bytes).expect("synthetic test bytes must parse");
        assert_eq!(log.combat_events.len(), 2);
        assert_eq!(log.stats.discarded.invalid_health_update, 1);
        // 保留的 health 行：105000→0 归零（bad 行丢弃、不参与归零）；
        // 末事件 500000-105000=395000
        assert_eq!(log.combat_events[0].time, 0);
        assert_eq!(log.log_start_offset, 105000);
        assert_eq!(log.log_end_time, 395000);
    }

    #[test]
    fn agent_change_collected_but_not_in_stream() {
        let change = combat_rev1(
            1000, 0xAAAABBBBCCCCDDDD, 0x1111222233334444, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, StateChange::AgentChange as u8, 0, 0, 0, 0,
        );
        let mut bytes = header(1);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&change);
        bytes.extend_from_slice(&plain_combat_rev1(300000));
        bytes.extend_from_slice(&plain_combat_rev1(305000));
        let log = parse_evtc_log(&bytes).expect("synthetic test bytes must parse");
        // AgentChange 在 IsValid ① 过滤，但重定向表先收集
        assert_eq!(log.combat_events.len(), 2);
        assert_eq!(
            log.agent_redirection.get(&0xAAAABBBBCCCCDDDD),
            Some(&0x1111222233334444)
        );
        assert_eq!(log.stats.discarded.unsupported_state_change, 1);
    }

    #[test]
    fn squad_combat_end_trims_to_extensions() {
        // boss 日志（id != 2）遇 SquadCombatEnd(非 Map 型) → stopAtLogEndEvent=0…
        // 构造：id=3 → stopAtLogEndEvent=-1；先 SquadCombatStart(Generic) → stop=0
        let mut bytes = header(1);
        bytes[13..15].copy_from_slice(&3u16.to_le_bytes()); // boss 触发 id
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let start = combat_rev1(100000, 0, LogType::Generic as u64, 100, 200, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, StateChange::SquadCombatStart as u8, 0, 0, 0, 0);
        let end = combat_rev1(200000, 0, 1, 100, 200, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, StateChange::SquadCombatEnd as u8, 0, 0, 0, 0);
        let after = plain_combat_rev1(300000);
        bytes.extend_from_slice(&start);
        bytes.extend_from_slice(&end);
        bytes.extend_from_slice(&after);
        let log = parse_evtc_log(&bytes).expect("synthetic test bytes must parse");
        // SquadCombatStart/End 行保留；后续普通行被裁
        assert_eq!(log.combat_events.len(), 2);
        assert_eq!(log.stats.discarded.after_log_end, 1);
    }

    #[test]
    fn extension_registration_and_keep() {
        // 首元事件:Extension + pad=0 + SrcAgent 含 sig/rev → 注册后自身弃用
        let mut meta = combat_rev1(500, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, StateChange::Extension as u8, 0, 0, 0, 0);
        // SrcAgent = rev<<32 | sig (HealingStats 0x9c9b3c99, rev 1)
        let src = (1u64 << 32) | 0x9c9b3c99u64;
        meta[8..16].copy_from_slice(&src.to_le_bytes());
        // 后续行:Extension + pad=sig → 保留
        let row = combat_rev1(900, 0xAA, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, StateChange::Extension as u8, 0, 0, 0, 0x9c9b3c99);
        let mut bytes = header(1);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&meta);
        bytes.extend_from_slice(&row);
        bytes.extend_from_slice(&plain_combat_rev1(400000));
        bytes.extend_from_slice(&plain_combat_rev1(405000));
        let log = parse_evtc_log(&bytes).expect("synthetic test bytes must parse");
        assert_eq!(log.enabled_extensions, vec![0x9c9b3c99]);
        assert_eq!(log.combat_events.len(), 3); // extension 行 + combat 行×2
        assert_eq!(log.combat_events[0].state_change, StateChange::Extension);
        assert_eq!(log.stats.discarded.extension_metadata, 1);
        // extension 行时间也归零（HasTime(enabled)=true，二次归零：900-400000）
        assert_eq!(log.combat_events[0].time, 900 - 400000);
    }
}
