//! EVTC 原始模型：header / agent / skill / combat item 与 ParseCombatList 产出。
//!
//! 对齐 `GW2EIEvtcParser` 的快照语义：
//! - combat item 是「ParseCombatList 处理后的行」（IsValid 过滤 + 时间归零已应用），
//!   字段级仍是原始字节值 —— 事件类型化（转成语义事件）在 `gw2ei-model` 做。
//! - 枚举在构造时即经 `from_byte` 折叠（对齐 C# `CombatItem` 构造器），
//!   同时保留 `*_raw` 原始字节（C# 侧经 `Get*` 折叠后原始值不可观测，
//!   保留 raw 便于对拍与调试）。
//! - 位判定（`SrcIsAgent`/`DstIsAgent`/`HasTime`/`IsEffect`/`IsMissile`/
//!   `IsExtension`）逐字对齐 `CombatItem.cs:151-330`。这些判定要交给
//!   `CompleteAgentsAndLogData`（P2），因此与事件化分发的实现放在一起。

use std::collections::BTreeMap;

use crate::enums::{Activation, BuffRemove, Iff, LogType, StateChange};

/// 解析设置。字段与语义对齐 `EvtcParserSettings`（ParserSettings.cs），
/// 构造时 `Math.Max(传参, 下限)` —— 下限即 `ParserHelper` 常量
/// `MinimumInCombatDuration = 2200` / `MinimumFileSizeMB = 100`。
#[derive(Debug, Clone, Copy)]
pub struct ParserSettings {
    /// 日志过短阈值（毫秒），下限 2200。
    pub too_short_limit_ms: i64,
    /// 解压后体积上限（MB），下限 100。
    pub too_big_limit_mb: u64,
    /// 是否解析扩展行（arcdps 扩展通道）。false 时所有 extension 行丢弃。
    pub parse_extensions: bool,
}

impl Default for ParserSettings {
    fn default() -> Self {
        Self {
            too_short_limit_ms: 2_200,
            too_big_limit_mb: 100,
            parse_extensions: true,
        }
    }
}

impl ParserSettings {
    /// 对齐 `EvtcParserSettings` 构造器：`TooShortLimit = Max(传参, 2200)`、
    /// `TooBigLimit = Max(传参, 100)`（ParserSettings.cs:22-25）。
    pub fn new(too_short_limit_ms: i64, too_big_limit_mb: u64) -> Self {
        Self {
            too_short_limit_ms: too_short_limit_ms.max(2_200),
            too_big_limit_mb: too_big_limit_mb.max(100),
            parse_extensions: true,
        }
    }
}

/// 一份 EVTC 日志的完整解析产出（对应 C# `ParseLog(Stream)` 在
/// `CompleteAgentsAndLogData` 之前的中间态，EvtcParser.cs:353-371）。
#[derive(Debug, Clone)]
pub struct EvtcRawLog {
    /// header 12B 版本串的整数部分（`EVTC` 之后、TryParse 成功才非错）。
    pub header_build: i32,
    /// 归档结构 revision（header[12]；非 0 一律按 rev1 读取，C# 不校验）。
    pub revision: u8,
    /// 触发 ID（header[13] u16；boss 遭遇 id，Instance=2）。
    pub id: u16,
    /// agent 区原始条目（未链接、未补缺 —— P2 `CompleteAgentsAndLogData`）。
    pub agents: Vec<EvtcRawAgent>,
    /// skill 区条目。
    pub skills: Vec<EvtcRawSkill>,
    /// 过滤 + 时间归零后的 combat 行（流序），供 P2 事件化。
    pub combat_events: Vec<EvtcCombatItem>,
    /// ArcDPS agent 重定向表（`StateChange.AgentChange` 行收集，不进流）。
    /// C# 侧为 `Dictionary<ulong, ulong>`；此处用 `BTreeMap` 保证迭代序稳定。
    pub agent_redirection: BTreeMap<u64, u64>,
    /// 已启用的扩展签名（EI 内置 HealingStats = 0x9c9b3c99，rev 1/2）。
    pub enabled_extensions: Vec<u32>,
    /// 首个 MapID 事件的 map id（无则 -1）。
    pub map_id: i32,
    /// 首个 HasTime 事件的原始时间（时间归零的减数，毫秒）。
    pub log_start_offset: i64,
    /// 最后一个 HasTime 事件归零后的时间（毫秒）。
    pub log_end_time: i64,
    /// ArcBuild 流内修订后的 arc 版本（Build, Revision）。C# `EvtcVersionEvent`：
    /// Build 初始 = header 值，ArcBuild 事件解析失败时回退 header 值、Revision=0；
    /// Revision 初始 -1 表示未遇到 ArcBuild。此处以 `Option` 语义存最终态：
    /// `(build, revision)`。
    pub arc_version: (i32, i32),
    /// GW2 build（最后一个非 0 的 GWBuild 事件值）。
    pub gw2_build: u64,
    /// 解析统计（原始计数 + 丢弃计数，按过滤原因分类）。
    pub stats: ParseStats,
}

/// 原始 agent 条目（EvtcParser.cs:544-587 读取序）。
/// 名字按 **原始 68B** 保留（C# `GetString(reader, 68, nullTerminated: false)`，
/// agent 名不按 `\0` 截断；`\0` 分隔的角色名分段由 P2 `AgentItem` 处理）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvtcRawAgent {
    pub address: u64,
    /// profession 字段（玩家为 1-9 职业 id；NPC/Gadget 时高 16 位为 0xffff 语义）。
    pub prof: u32,
    /// is_elite 字段。`u32::MAX` 表示非玩家（NPC/Gadget，见 `agent_kind`）。
    pub is_elite: u32,
    pub toughness: u16,
    pub concentration: u16,
    pub healing: u16,
    /// hitbox width **raw 值**（C# 在 AgentItem 构建时 ×2，且玩家强设 48×240 —— P2）。
    pub hitbox_width_raw: u16,
    pub condition: u16,
    /// hitbox height **raw 值**（同上 ×2 在 P2）。
    pub hitbox_height_raw: u16,
    /// 名字原始 68 字节。
    pub name: [u8; 68],
}

/// 轻量 agent 类型判定，供 P1 事件化（cast 配对判定玩家等）。
/// 对齐 `GW2APIController.GetSpec`（GW2APIController.cs:114-166）的本地可判定部分：
/// `is_elite == 0xFFFFFFFF` → 非玩家（prof 高 16 位全 1 为 Gadget，否则 NPC）；
/// 其余（含查 API 表失败的 Unknown）C# 走 `AgentItem.AgentType.Player`。
/// 注意：这与「name 前缀 NPC/GDG」的直觉不同，以 C# 源码为准。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawAgentKind {
    Player,
    Npc,
    Gadget,
}

impl EvtcRawAgent {
    /// 名字按首个 `\0` 截断的视图（不含 NUL）。对齐 C# 后续按 `\0` 拆段的输入。
    pub fn name_cstr(&self) -> &[u8] {
        let nul = self.name.iter().position(|&b| b == 0).unwrap_or(68);
        &self.name[..nul]
    }

    /// 名字完整 68B 的 UTF-8 lossy 解码（对齐 C# `GetString(false)`：整个缓冲
    /// 解码，非法字节替换 U+FFFD、`\0` 原样保留在串中）。
    pub fn name_full_lossy(&self) -> String {
        String::from_utf8_lossy(&self.name).into_owned()
    }

    /// agent 类型判定（见 `RawAgentKind` 文档）。
    pub fn agent_kind(&self) -> RawAgentKind {
        if self.is_elite == u32::MAX {
            if self.prof & 0xffff_0000 == 0xffff_0000 {
                RawAgentKind::Gadget
            } else {
                RawAgentKind::Npc
            }
        } else {
            RawAgentKind::Player
        }
    }
}

/// 原始 skill 条目（EvtcParser.cs:599-618）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvtcRawSkill {
    /// skill id。C# `ReadInt32` 读出后存 `long`；此处以 u32 同比特位存储
    /// （负 id 的位型在 P2 `SkillItem`/JSON 对齐层再解释，`SkillData.Get`
    /// 以 long key 存取，负 id 是合法的 arc 内置 id 如 WeaponSwap=-2）。
    pub id: u32,
    /// 名字：64B 按首个 `\0` 截断后 UTF-8 解码。
    /// 空名 **不回退**（与 SBR evtc-format 的 `id.to_string()` 兜底不同）：
    /// C# `SkillData.Add` 直接把空串存入 `SkillItem`，无名字兜底；后续 API/
    /// override 覆盖是 P2+ 语义。
    pub name: String,
}

/// 丢弃统计：按 `IsValid`（EvtcParser.cs:926-976）与 keepOnlyExtensionEvents
/// 裁尾（:854-857, :887-890）的过滤原因分类计数。C# 只累计一个整数，
/// 分类计数是本重构的可观测性增强（数值对齐 C#：各分类之和 = discarded 总数）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiscardStats {
    /// ① 不受支持的 state change（`IsSupportedStateChange` 排除集）。
    pub unsupported_state_change: u64,
    /// ② 事件涉 agent 但 map id 已变更（`mapID != currentMapID`）。
    pub map_mismatch: u64,
    /// ③ instance 日志（id == 2）里的 `BuffInitial`。
    pub instance_buff_initial: u64,
    /// ④ `HealthUpdate` 行血量 > 200%。
    pub invalid_health_update: u64,
    /// ⑤a `ParseExtensions = false` 时全部 extension 行。
    pub extensions_disabled: u64,
    /// ⑤b 首个 Extension 元事件（Pad==0，注册 handler 后自身弃用）。
    pub extension_metadata: u64,
    /// ⑤c extension 行 Pad 不在已注册签名表内。
    pub unknown_extension_pad: u64,
    /// ⑥ 空事件（instid/agent 全 0 且 IFF Unknown 且非 effect/missile）。
    pub empty_event: u64,
    /// `keepOnlyExtensionEvents` 裁尾丢弃的非 extension 行。
    pub after_log_end: u64,
}

impl DiscardStats {
    pub fn total(&self) -> u64 {
        self.unsupported_state_change
            + self.map_mismatch
            + self.instance_buff_initial
            + self.invalid_health_update
            + self.extensions_disabled
            + self.extension_metadata
            + self.unknown_extension_pad
            + self.empty_event
            + self.after_log_end
    }
}

/// ParseCombatList 解析统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParseStats {
    /// agent 区原始条数（agent_count 字段）。
    pub raw_agent_count: u32,
    /// skill 区原始条数。
    pub raw_skill_count: u32,
    /// combat 区原始条数（`(Length - Position) / 64`，尾数静默忽略）。
    pub raw_combat_count: u64,
    /// 通过全部过滤、留在列表中的行数。
    pub kept_combat_count: usize,
    /// ArcBuild 版本行被消费数（SetFromCombatItem 后 continue，不进流也
    /// 不计丢弃 —— C# EvtcParser.cs:860-864 语义）。
    pub arc_build_consumed: u64,
    /// 按原因分类的丢弃计数。
    pub discarded: DiscardStats,
}

/// 64B combat 行（两种 revision 布局统一后的字段面）。
///
/// rev0 差异（EvtcParser.cs:627-706）：`overstack_value`/`skill_id` 为 u16、
/// 无 `dst_master_instid`（42-50 共 9B 垃圾跳过，置 0）、偏移 63 有 1B 垃圾
/// 跳过、`pad` 恒 0（rev0 无扩展能力）。
///
/// 字段改动语义（时间归零）在 `log_reader.rs`；revision 布局只影响读取。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvtcCombatItem {
    /// 毫秒。ParseCombatList 时间归零后：HasTime 事件为 (原始 - log_start_offset)；
    /// 无时间事件保留原始值（靠 P2 `OffsetEvtcData` 修正）。
    pub time: i64,
    pub src_agent: u64,
    pub dst_agent: u64,
    pub value: i32,
    pub buff_dmg: i32,
    pub overstack_value: u32,
    /// 同 C# `SkillID`：u32 原值（rev0 由 u16 扩展）。
    pub skill_id: u32,
    pub src_instid: u16,
    pub dst_instid: u16,
    pub src_master_instid: u16,
    pub dst_master_instid: u16,
    pub iff: Iff,
    pub iff_raw: u8,
    /// buff 标志字节（0=直接伤害行, 非 0=症状/间接伤害行 —— 新世代判定）。
    pub buff: u8,
    pub result: u8,
    pub activation: Activation,
    pub activation_raw: u8,
    pub buff_remove: BuffRemove,
    pub buff_remove_raw: u8,
    pub is_ninety: u8,
    pub is_fifty: u8,
    pub is_moving: u8,
    pub state_change: StateChange,
    pub state_change_raw: u8,
    pub is_flanking: u8,
    pub is_shields: u8,
    pub is_offcycle: u8,
    /// rev1: 4B 扩展签名/事件负载；rev0: 恒 0。
    pub pad: u32,
}

/// C# `CombatItem.Pad1..Pad4`：pad 的低位起的 4 个字节（CombatItem.cs:104-111，
/// 原 unsafe 拆字节，safe Rust 用 `to_le_bytes` 等价）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PadBytes {
    pub b1: u8,
    pub b2: u8,
    pub b3: u8,
    pub b4: u8,
}

impl EvtcCombatItem {
    pub fn pad_bytes(&self) -> PadBytes {
        let b = self.pad.to_le_bytes();
        PadBytes {
            b1: b[0],
            b2: b[1],
            b3: b[2],
            b4: b[3],
        }
    }

    /// CombatItem.cs:43：扩展行（Extension 状态通道）。
    pub fn is_extension(&self) -> bool {
        self.state_change == StateChange::Extension
            || self.state_change == StateChange::ExtensionCombat
    }

    /// CombatItem.cs:50：effect 系 state change（P1 不做这些事件的类型化）。
    pub fn is_effect(&self) -> bool {
        matches!(
            self.state_change,
            StateChange::Effect_51
                | StateChange::Effect_45
                | StateChange::EffectAgentCreate
                | StateChange::EffectAgentRemove
                | StateChange::EffectGroundCreate
                | StateChange::EffectGroundRemove
                | StateChange::EffectMissileCreate
        )
    }

    /// CombatItem.cs:51：missile 系 state change。
    pub fn is_missile(&self) -> bool {
        matches!(
            self.state_change,
            StateChange::MissileCreate | StateChange::MissileLaunch | StateChange::MissileRemove
        )
    }

    /// CombatItem.cs:53-59：需要第一遍先行处理的 metadata 事件。
    pub fn is_essential_metadata(&self) -> bool {
        matches!(
            self.state_change,
            StateChange::IDToGUID
                | StateChange::Language
                | StateChange::GWBuild
                | StateChange::InstanceStart
                | StateChange::LogNPCUpdate
                | StateChange::FractalScale
                | StateChange::MapID
                | StateChange::RuleSet
                | StateChange::Tick
                | StateChange::SquadCombatEnd
                | StateChange::SquadCombatStart
                | StateChange::TickRate
                | StateChange::WvWTeams
        )
    }

    /// CombatItem.cs:242-294 `SrcIsAgent`：SrcAgent 字段承载 agent 地址的
    /// state change 白名单（不依赖扩展表）。
    pub fn src_is_agent(&self) -> bool {
        self.state_change == StateChange::Combat
            || self.state_change == StateChange::EnterCombat
            || self.state_change == StateChange::ExitCombat
            || self.state_change == StateChange::ChangeUp
            || self.state_change == StateChange::ChangeDead
            || self.state_change == StateChange::ChangeDown
            || self.state_change == StateChange::Spawn
            || self.state_change == StateChange::Despawn
            || self.state_change == StateChange::HealthUpdate
            || self.state_change == StateChange::WeaponSwap
            || self.state_change == StateChange::MaxHealthUpdate
            || self.state_change == StateChange::PointOfView
            || self.state_change == StateChange::BuffInitial
            || self.is_geographical()
            || self.state_change == StateChange::TeamChange
            || self.state_change == StateChange::AttackTarget
            || self.state_change == StateChange::Targetable
            || self.state_change == StateChange::StackActive
            || self.state_change == StateChange::StackDeactive
            || self.state_change == StateChange::Guild
            || self.state_change == StateChange::BreakbarState
            || self.state_change == StateChange::BreakbarPercent
            || self.state_change == StateChange::Marker
            || self.state_change == StateChange::BarrierUpdate
            || self.state_change == StateChange::Last90BeforeDown
            || self.state_change == StateChange::Effect_45
            || self.state_change == StateChange::Effect_51
            || self.state_change == StateChange::EffectGroundCreate
            || self.state_change == StateChange::EffectAgentCreate
            || self.state_change == StateChange::Glider
            || self.state_change == StateChange::StunBreak
            || self.state_change == StateChange::MissileCreate
            || self.state_change == StateChange::MissileLaunch
            || self.state_change == StateChange::MissileRemove
            || self.state_change == StateChange::AnimationStart
            || self.state_change == StateChange::AnimationStop
            || self.state_change == StateChange::BuffRemoveAll
            || self.state_change == StateChange::BuffRemoveSingle
            || self.state_change == StateChange::BuffApply
            || self.state_change == StateChange::Transformation
            || self.state_change == StateChange::StealthChange
            || self.state_change == StateChange::GadgetAnimation
            || self.state_change == StateChange::GadgetNameVisible
            || self.state_change == StateChange::EffectMissileCreate
            || self.state_change == StateChange::GadgetCaptureOutlineShow
            || self.state_change == StateChange::GadgetCaptureSplitPercent
            || self.state_change == StateChange::GadgetCaptureOutlineHide
            || self.state_change == StateChange::GadgetCaptureOutlinePoint
            || self.state_change == StateChange::Jump
    }

    /// CombatItem.cs:305-321 `DstIsAgent`：DstAgent 承载 agent 地址的白名单。
    pub fn dst_is_agent(&self) -> bool {
        self.state_change == StateChange::Combat
            || self.state_change == StateChange::AttackTarget
            || self.state_change == StateChange::BuffInitial
            || self.state_change == StateChange::Effect_45
            || self.state_change == StateChange::LogNPCUpdate
            || self.state_change == StateChange::Effect_51
            || self.state_change == StateChange::EffectAgentCreate
            || self.state_change == StateChange::MissileLaunch
            || self.state_change == StateChange::AnimationStart
            || self.state_change == StateChange::BuffRemoveAll
            || self.state_change == StateChange::BuffRemoveSingle
            || self.state_change == StateChange::BuffApply
            || self.state_change == StateChange::BuffChange
    }

    /// CombatItem.cs:151-167 `HasTime`：参与时间归零的事件（含扩展时另见
    /// `has_time_with_extensions`）。HealingStats 扩展行 HasTime=true
    /// （HealingStatsExtensionHandler.cs:207-210），但扩展注册前不参与 ——
    /// 处理次序见 `log_reader.rs`。
    pub fn has_time(&self) -> bool {
        self.src_is_agent()
            || self.dst_is_agent()
            || self.state_change == StateChange::Reward
            || self.state_change == StateChange::TickRate
            || self.state_change == StateChange::SquadMarker
            || self.state_change == StateChange::SquadCombatStart
            || self.state_change == StateChange::SquadCombatEnd
            || self.state_change == StateChange::EffectAgentRemove
            || self.state_change == StateChange::EffectGroundRemove
            || self.state_change == StateChange::MapID
            || self.state_change == StateChange::MapChange
            || self.state_change == StateChange::WvWObjectiveStatus
            || self.state_change == StateChange::Tick
    }

    /// CombatItem.cs:44-47：位置/传送系（Movement 抽象的事件基底）。
    pub fn is_position(&self) -> bool {
        matches!(
            self.state_change,
            StateChange::Position | StateChange::Teleport
        )
    }

    pub fn is_geographical(&self) -> bool {
        self.is_position()
            || self.state_change == StateChange::Rotation
            || self.state_change == StateChange::Velocity
    }
}

/// EI 内置扩展表（ExtensionHelper.cs:11-35 + HealingStatsExtensionHandler.cs:10）：
/// 唯一内置为 HealingStats `0x9c9b3c99`（rev 1/2 支持）。P1 只复刻
/// 「注册/保留行」语义，extension 行的内容解析在后续阶段。
pub const EXT_HEALING_STATS: u32 = 0x9c9b3c99;

/// 解析扩展头：sig = SrcAgent 低 32 位、rev = 24 位 @32（ExtensionHelper.cs:6-8）。
/// 返回 `(sig, rev)`。
pub fn parse_extension_header(src_agent: u64) -> (u32, u32) {
    let sig = (src_agent & 0x0000_0000_FFFF_FFFF) as u32;
    let rev = ((src_agent >> 32) & 0x00FF_FFFF) as u32;
    (sig, rev)
}

/// EI 扩展支持判定（ExtensionHelper.cs:15-34 的 switch 表）。
pub fn extension_supported(sig: u32, rev: u32) -> bool {
    match sig {
        EXT_HEALING_STATS => matches!(rev, 1 | 2),
        _ => false,
    }
}

/// arc 内置技能 id 常量（SkillIDs.cs:85-119 相关子集，P1 cast 配对/分派用）。
pub mod skill_ids {
    use crate::enums::arc_builds;

    /// SkillData.DodgeID：build >= 20220304 后 arc 的 dodge id（SkillData.cs:17）。
    pub const ARC_DODGE_20220307: u32 = 23275;
    /// 旧 dodge id（build < 20220304）。
    pub const ARC_DODGE_LEGACY: u32 = 65001;
    /// 泛用破蔑 id（无专属技能的破蔑伤害行用）。
    pub const ARC_GENERIC_BREAKBAR_20220307: u32 = 23276;
    pub const ARC_GENERIC_BREAKBAR_LEGACY: u32 = 65002;
    /// WeaponSwap 合成技能（SkillIDs.cs:19）。
    pub const WEAPON_SWAP: i64 = -2;
    /// 复活技能（AnimatedCastEvent 跳过复活的状态判定）。
    pub const RESURRECT: i64 = 1066;
    /// emote / gadget interact / bundle pick-up 三条合成动画行（SkillIDs.cs:113-119），
    /// build >= 20260318 后按 skill id 分派为 EmoteEvent/GadgetInteractEvent/BundlePickUpEvent。
    pub const ARC_GENERIC_EMOTE: u32 = 23303;
    pub const ARC_GENERIC_GADGET_INTERACT: u32 = 23302;
    pub const ARC_GENERIC_PICK_UP: u32 = 23308;
    /// NoBuff（buff 无效化用，P3）。
    pub const NO_BUFF: i64 = i64::MIN;

    /// 对齐 `SkillItem.GetArcDPSCustomIDs`（SkillItem.cs:13-18）。
    pub fn dodge_and_breakbar_for_build(build: i32) -> (u32, u32) {
        if build >= arc_builds::INTERNAL_SKILL_IDS_CHANGE {
            (ARC_DODGE_20220307, ARC_GENERIC_BREAKBAR_20220307)
        } else {
            (ARC_DODGE_LEGACY, ARC_GENERIC_BREAKBAR_LEGACY)
        }
    }
}

/// 供工厂层使用的动作/类型辅助解码（值均直接来自字节字段）。
impl EvtcCombatItem {
    /// SquadCombatStart/End 行:日志类型在 DstAgent（SquadCombatStartEvent.cs:
    /// `GetLogType((int)DstAgent)` —— C# 先截断到 int 再与 `< Unknown` 比较）。
    pub fn squad_combat_log_type(&self) -> LogType {
        let low32 = (self.dst_agent & 0xFFFF_FFFF) as u32;
        LogType::from_csharp_int(low32 as i32)
    }
}
