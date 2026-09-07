//! 职业/专精枚举与本地解析。
//!
//! C# 侧 `EnterCombatEvent.Spec = ProfToSpec(agentData.GetSpec(Value, BuffDmg))`
//!（CombatStatusEvent.cs）：`GetSpec` 走 GW2 API 缓存（prof/elite → spec 名），
//! 名字再查 `ProfToSpecDictionary`。P1 不做 API 缓存，只实现
//! `GW2APIController.GetSpec`（GW2APIController.cs:114-166）的本地可判定部分：
//! - `elite == 0xFFFFFFFF`：非玩家（GDG/NPC）；
//! - `elite == 0`：基础职业 1-9 硬编码表；
//! - `elite == 1`：HoT 精英专精硬编码表；
//! - 其余（2017 后精英专精按 API 表）：P1 返回 `Unknown`，P2 引入
//!   `SpecList.json` 缓存后补齐。

use gw2ei_parse::RawAgentKind;

/// `ParserHelper.Spec`（ParserHelper.cs:67-82）的枚举值（仅名字语义，P1 不
/// 依赖数值序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)] // C# 成员名逐一保留
pub enum Spec {
    // 职业（按基础职业字母序 + 每个职业的精英专精）
    Elementalist,
    Tempest,
    Weaver,
    Catalyst,
    Evoker,
    Engineer,
    Scrapper,
    Holosmith,
    Mechanist,
    Amalgam,
    Guardian,
    Dragonhunter,
    Firebrand,
    Willbender,
    Luminary,
    Mesmer,
    Chronomancer,
    Mirage,
    Virtuoso,
    Troubadour,
    Necromancer,
    Reaper,
    Scourge,
    Harbinger,
    Ritualist,
    Ranger,
    Druid,
    Soulbeast,
    Untamed,
    Galeshot,
    Revenant,
    Herald,
    Renegade,
    Vindicator,
    Conduit,
    Thief,
    Daredevil,
    Deadeye,
    Specter,
    Antiquary,
    Warrior,
    Berserker,
    Spellbreaker,
    Bladesworn,
    Paragon,
    // 非玩家
    Npc,
    Gadget,
    Unknown,
}

impl Spec {
    pub fn csharp_name(self) -> &'static str {
        match self {
            Spec::Elementalist => "Elementalist",
            Spec::Tempest => "Tempest",
            Spec::Weaver => "Weaver",
            Spec::Catalyst => "Catalyst",
            Spec::Evoker => "Evoker",
            Spec::Engineer => "Engineer",
            Spec::Scrapper => "Scrapper",
            Spec::Holosmith => "Holosmith",
            Spec::Mechanist => "Mechanist",
            Spec::Amalgam => "Amalgam",
            Spec::Guardian => "Guardian",
            Spec::Dragonhunter => "Dragonhunter",
            Spec::Firebrand => "Firebrand",
            Spec::Willbender => "Willbender",
            Spec::Luminary => "Luminary",
            Spec::Mesmer => "Mesmer",
            Spec::Chronomancer => "Chronomancer",
            Spec::Mirage => "Mirage",
            Spec::Virtuoso => "Virtuoso",
            Spec::Troubadour => "Troubadour",
            Spec::Necromancer => "Necromancer",
            Spec::Reaper => "Reaper",
            Spec::Scourge => "Scourge",
            Spec::Harbinger => "Harbinger",
            Spec::Ritualist => "Ritualist",
            Spec::Ranger => "Ranger",
            Spec::Druid => "Druid",
            Spec::Soulbeast => "Soulbeast",
            Spec::Untamed => "Untamed",
            Spec::Galeshot => "Galeshot",
            Spec::Revenant => "Revenant",
            Spec::Herald => "Herald",
            Spec::Renegade => "Renegade",
            Spec::Vindicator => "Vindicator",
            Spec::Conduit => "Conduit",
            Spec::Thief => "Thief",
            Spec::Daredevil => "Daredevil",
            Spec::Deadeye => "Deadeye",
            Spec::Specter => "Specter",
            Spec::Antiquary => "Antiquary",
            Spec::Warrior => "Warrior",
            Spec::Berserker => "Berserker",
            Spec::Spellbreaker => "Spellbreaker",
            Spec::Bladesworn => "Bladesworn",
            Spec::Paragon => "Paragon",
            Spec::Npc => "NPC",
            Spec::Gadget => "GDG",
            Spec::Unknown => "Unknown",
        }
    }

    /// `ParserHelper.SpecToBaseSpec` 的本地子集：精英专精 → 基础职业。
    /// 非玩家/Unknown 原样返回（ParserHelper.cs:394-397 的
    /// `TryGetValue ?? spec` 语义）。
    pub fn base_spec(self) -> Spec {
        use Spec::*;
        match self {
            Galeshot | Untamed | Soulbeast | Druid => Ranger,
            Amalgam | Mechanist | Holosmith | Scrapper => Engineer,
            Antiquary | Specter | Deadeye | Daredevil => Thief,
            Evoker | Catalyst | Weaver | Tempest => Elementalist,
            Troubadour | Virtuoso | Mirage | Chronomancer => Mesmer,
            Ritualist | Harbinger | Scourge | Reaper => Necromancer,
            Paragon | Bladesworn | Spellbreaker | Berserker => Warrior,
            Luminary | Willbender | Firebrand | Dragonhunter => Guardian,
            Conduit | Vindicator | Renegade | Herald => Revenant,
            other => other,
        }
    }
}

/// 本地 spec 判定（对齐 `GW2APIController.GetSpec`，见模块文档）。
pub fn spec_from_prof_elite(prof: u32, elite: u32) -> Spec {
    use Spec::*;
    if elite == u32::MAX {
        // 非玩家：GDG = prof 高 16 位全 1，否则 NPC。
        return if prof & 0xFFFF_0000 == 0xFFFF_0000 {
            Gadget
        } else {
            Npc
        };
    }
    match elite {
        // 旧方式 - 基础职业（GW2APIController.cs:122-137）
        0 => match prof {
            1 => Guardian,
            2 => Warrior,
            3 => Engineer,
            4 => Ranger,
            5 => Thief,
            6 => Elementalist,
            7 => Mesmer,
            8 => Necromancer,
            9 => Revenant,
            _ => Unknown,
        },
        // 旧方式 - HoT 精英专精（GW2APIController.cs:139-154）
        1 => match prof {
            1 => Dragonhunter,
            2 => Berserker,
            3 => Scrapper,
            4 => Druid,
            5 => Daredevil,
            6 => Tempest,
            7 => Chronomancer,
            8 => Reaper,
            9 => Herald,
            _ => Unknown,
        },
        // 当前方式：按 spec id 查 API SpecList —— P2（Unknown 在 C# 侧
        // GetSpec 返回 "Unknown" 后仍按 Player 类型处理，Spec 语义不丢）。
        _ => Unknown,
    }
}

impl From<RawAgentKind> for Spec {
    fn from(kind: RawAgentKind) -> Self {
        match kind {
            RawAgentKind::Player => Spec::Unknown,
            RawAgentKind::Npc => Spec::Npc,
            RawAgentKind::Gadget => Spec::Gadget,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_spec_decoding() {
        assert_eq!(
            spec_from_prof_elite(0xFFFF_1234, u32::MAX),
            Spec::Gadget
        );
        assert_eq!(spec_from_prof_elite(1234, u32::MAX), Spec::Npc);
        assert_eq!(spec_from_prof_elite(1, 0), Spec::Guardian);
        assert_eq!(spec_from_prof_elite(9, 0), Spec::Revenant);
        assert_eq!(spec_from_prof_elite(2, 1), Spec::Berserker);
        assert_eq!(spec_from_prof_elite(0, 0), Spec::Unknown);
        // 现代精英专精（API 表）P1 未知
        assert_eq!(spec_from_prof_elite(1, 62), Spec::Unknown);
    }

    #[test]
    fn base_spec_map() {
        assert_eq!(Spec::Dragonhunter.base_spec(), Spec::Guardian);
        assert_eq!(Spec::Catalyst.base_spec(), Spec::Elementalist);
        assert_eq!(Spec::Npc.base_spec(), Spec::Npc);
        assert_eq!(Spec::Unknown.base_spec(), Spec::Unknown);
    }
}

/// 外部内容目录提供的 spec 判定(prof, elite) → Spec。
/// C# `GW2APIController.GetSpec`(prof, elite)走 API SpecList 缓存;
/// Rust 由 `gw2ei-json` 的 Content 目录实现(SpecList.json),默认无表版本
/// 退化为 `spec_from_prof_elite`(本地硬编码子集)。
pub trait SpecCatalog {
    fn spec_of(&self, prof: u32, elite: u32) -> Spec;
}

/// 无 SpecList 的默认实现(仅本地硬编码表)。
pub struct NoSpecCatalog;

impl SpecCatalog for NoSpecCatalog {
    fn spec_of(&self, prof: u32, elite: u32) -> Spec {
        spec_from_prof_elite(prof, elite)
    }
}
