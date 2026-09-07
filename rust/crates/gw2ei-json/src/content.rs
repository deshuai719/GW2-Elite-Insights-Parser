//! Content 资产层:GW2 API 缓存(SkillList/SpecList/MapList)、Buff 注册表
//! (buff-table.json,extract-buff-table.py 从 C# 抽取)、SkillItemOverrides
//! (skill-overrides.json)。资产缺失 = 显式错误,不静默降级。
//!
//! 对应 C#:BuffsContainer.cs + GW2APIController(GetSpec/GetAPISkill) +
//! SkillItemOverrides.cs + SkillData(仅名字面)。

use std::collections::BTreeMap;
use std::path::Path;

use gw2ei_model::Spec;

use crate::BuildError;

// ===== Buff 定义(buff-table.json 行)=====

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuffClassification {
    Condition,
    Boon,
    Offensive,
    Defensive,
    Support,
    Debuff,
    Gear,
    Other,
    Enhancement,
    Nourishment,
    OtherConsumable,
    Hidden,
    Unknown,
}

impl BuffClassification {
    fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "Condition" => Self::Condition,
            "Boon" => Self::Boon,
            "Offensive" => Self::Offensive,
            "Defensive" => Self::Defensive,
            "Support" => Self::Support,
            "Debuff" => Self::Debuff,
            "Gear" => Self::Gear,
            "Other" => Self::Other,
            "Enhancement" => Self::Enhancement,
            "Nourishment" => Self::Nourishment,
            "OtherConsumable" => Self::OtherConsumable,
            "Hidden" => Self::Hidden,
            "Unknown" => Self::Unknown,
            _ => return None,
        })
    }
    /// JsonLogBuilder.BuildBuffDesc switch(Unknown/Hidden 无分支 → 键省略)。
    pub fn json_name(self) -> Option<&'static str> {
        Some(match self {
            Self::Condition => "Condition",
            Self::Boon => "Boon",
            Self::Offensive => "Offensive",
            Self::Defensive => "Defensive",
            Self::Support => "Support",
            Self::Debuff => "Debuff",
            Self::OtherConsumable => "Other Consumable",
            Self::Nourishment => "Nourishment",
            Self::Enhancement => "Enhancement",
            Self::Gear => "Gear",
            Self::Other => "Other",
            Self::Hidden | Self::Unknown => return None,
        })
    }
}

/// 一条 `new Buff(...)`(buff-table.json 保持 C# 源顺序 —— 同 id 多条时首条
/// 有效,同 BuffsContainer/BuffsByIDs 的 GroupBy-first)。
#[derive(Debug, Clone)]
pub struct BuffDef {
    pub id: i64,
    pub name: String,
    pub classification: BuffClassification,
    pub stack_type: String, // C# enum 名:Queue/Stacking/.../Unknown
    pub capacity: i64,
    pub icon: String,
    pub sources: Vec<String>,
    pub min_gw2_build: u64,
    pub max_gw2_build: u64,
    pub min_evtc_build: i64,
    pub max_evtc_build: i64,
    /// 所属静态列表(BuffsContainer.AllBuffs 的成员名;动态食物/强化合成用)。
    pub list: String,
}

impl BuffDef {
    /// `Buff.Type == Intensity`(Buff.cs:45-56) → JSON `stacking`。
    pub fn is_intensity(&self) -> bool {
        matches!(
            self.stack_type.as_str(),
            "Stacking" | "StackingUniquePerSrc" | "StackingConditionalLoss"
        )
    }
    /// `Buff.Available(combatData)`(Buff.cs:262-276):gw2 build 与 evtc build
    /// 窗口双开区间。
    pub fn available(&self, gw2_build: u64, evtc_build: i64) -> bool {
        self.min_gw2_build <= gw2_build
            && gw2_build < self.max_gw2_build
            && self.min_evtc_build <= evtc_build
            && evtc_build < self.max_evtc_build
    }
}

// ===== Buff 注册表(某日志的激活集)=====

/// `BuffsContainer.BuffsByIDs` 的 Rust 面:available 过滤 + 同 id 取首 +
/// 动态 Nourishment/Enhancement 合成。
#[derive(Debug, Clone, Default)]
pub struct BuffRegistry {
    by_id: BTreeMap<i64, BuffDef>,
    /// available 且非 Hidden 的全量(分类查询用;含同 id 首条语义)。
    pub active: Vec<BuffDef>,
}

impl BuffRegistry {
    /// available 过滤 + 同 id 首条(C# BuffsByIDs:currentBuffs.GroupBy first)。
    /// `dynamic` 已按 C# BuffsContainer 的合成规则生成(调用方从 BuffInfo
    /// 事件构建)。
    fn build(all: &[BuffDef], gw2_build: u64, evtc_build: i64, dynamic: Vec<BuffDef>) -> Self {
        let mut active: Vec<BuffDef> = all
            .iter()
            .filter(|b| b.available(gw2_build, evtc_build))
            .cloned()
            .collect();
        active.extend(dynamic);
        let mut by_id: BTreeMap<i64, BuffDef> = BTreeMap::new();
        for b in &active {
            by_id.entry(b.id).or_insert_with(|| b.clone());
        }
        BuffRegistry { by_id, active }
    }

    pub fn get(&self, id: i64) -> Option<&BuffDef> {
        self.by_id.get(&id)
    }
    /// `Classification == Condition`(SkillEvent.ConditionDamageBased)。
    pub fn is_condition(&self, id: i64) -> bool {
        self.by_id
            .get(&id)
            .map(|b| b.classification == BuffClassification::Condition)
            .unwrap_or(false)
    }
    /// Boon 分类 id 集合(Defense/Support 的 strip 统计用)。
    pub fn boon_ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.by_id.values().filter_map(|b| {
            (b.classification == BuffClassification::Boon).then_some(b.id)
        })
    }
    pub fn condition_ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.by_id.values().filter_map(|b| {
            (b.classification == BuffClassification::Condition).then_some(b.id)
        })
    }
}

// ===== GW2 API 缓存 =====

/// SkillList.json 的 GW2APISkill 子集(SkillItem ctor 用字段)。
#[derive(Debug, Clone, Default)]
pub struct ApiSkill {
    pub name: String,
    pub icon: String,
    pub ty: Option<String>,
    pub weapon_type: Option<String>,
    pub slot: Option<String>,
    pub professions: Vec<String>,
    pub categories: Option<Vec<String>>,
    pub description: Option<String>,
    pub flags: Option<Vec<String>>,
    pub dual_wield: Option<String>,
    /// API `bundle_skills`(Engineer kit 技能集;EngineerKitFinder 消费)。
    pub bundle_skills: Option<Vec<i64>>,
}

/// skill-overrides.json(SkillItemOverrides.cs)。
#[derive(Debug, Clone, Default)]
pub struct SkillOverrides {
    pub names: BTreeMap<i64, String>,
    pub icons: BTreeMap<i64, String>,
    /// NonCritableSkills:id → 该 build 起不可暴击(gw2Build >= build)。
    pub non_critable: BTreeMap<i64, u64>,
}

// ===== Content 总装 =====

/// 资产加载产物(不可变;加载一次复用)。
#[derive(Debug, Clone, Default)]
pub struct Content {
    pub buffs: Vec<BuffDef>,
    pub skills_api: BTreeMap<i64, ApiSkill>,
    pub overrides: SkillOverrides,
    pub spec_by_id: BTreeMap<i64, SpecLine>,
    pub map_names: BTreeMap<i64, String>,
    /// instant-cast finder 表(extract-instant-casts.py 产物;P3b 引擎输入)。
    pub instant_finders: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct SpecLine {
    pub name: String,
    pub profession: String,
    pub elite: bool,
}

const NOURISHMENT_ICON: &str = "https://render.guildwars2.com/file/779D3F0ABE5B46C09CFC57374DA8CC3A495F291C/436367.png"; // ItemImages.NourishmentEffect
const ENHANCEMENT_ICON: &str = "https://render.guildwars2.com/file/64976F59BF060718C9109D36C27AD0F40F06BB32/436368.png"; // ItemImages.EnhancementEffect
// SkillImages.cs MonsterSkill(注意 EI 用 render CDN 版而非 wiki 镜像)。
const DEFAULT_SKILL_ICON: &str = "https://render.guildwars2.com/file/1D55D34FB4EE20B1962E315245E40CA5E1042D0E/62248.png";

fn load_buff_table(p: &Path) -> Result<Vec<BuffDef>, BuildError> {
    let raw: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(p).map_err(|e| BuildError::Content(format!("read {p:?}: {e}")))?,
    )
    .map_err(|e| BuildError::Content(format!("parse {p:?}: {e}")))?;
    let arr = raw
        .get("buff")
        .and_then(|v| v.as_array())
        .ok_or_else(|| BuildError::Content("buff-table.json missing 'buff' array".into()))?;
    let mut out = Vec::with_capacity(arr.len());
    for b in arr {
        let cls = BuffClassification::from_str(
            b["classification"].as_str().unwrap_or("Unknown"),
        )
        .unwrap_or(BuffClassification::Unknown);
        out.push(BuffDef {
            id: b["id"].as_i64().unwrap_or(0),
            name: b["name"].as_str().unwrap_or("").to_string(),
            classification: cls,
            stack_type: b["stack_type"].as_str().unwrap_or("Unknown").to_string(),
            capacity: b["capacity"].as_i64().unwrap_or(1),
            icon: b["icon"].as_str().unwrap_or("").to_string(),
            sources: b["sources"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            min_gw2_build: b["min_gw2_build"].as_u64().unwrap_or(0),
            max_gw2_build: b["max_gw2_build"].as_u64().unwrap_or(u64::MAX),
            min_evtc_build: b["min_evtc_build"].as_i64().unwrap_or(i64::MIN),
            max_evtc_build: b["max_evtc_build"].as_i64().unwrap_or(i64::MAX),
            list: b["list"].as_str().unwrap_or("").to_string(),
        });
    }
    Ok(out)
}

/// Buff 注册表按日志计算:
/// 1. available(gw2Build, evtcBuild) 过滤 + 同 id 首条;
/// 2. 动态 Nourishment/Enhancement 合成(BuffsContainer.cs:108-140):
///    静态表(NormalFoods/AscendedFood | Utilities/Writs/SlayingPotions)成员
///    的 BuffInfoEvent CategoryByte 恰一个时,该类目下所有不在注册表内的
///    buff id 合成 "Unknown Nourishment {id}"/"Unknown Enhancement {id}"。
///
/// `buff_info`:log 内 BuffInfo 行(id → BuffInfoRow);id 参与判定的成员
/// 来自静态表自身(含 list 资格)。
pub fn registry_for(
    content: &Content,
    gw2_build: u64,
    evtc_build: i64,
    buff_info: &BTreeMap<u32, (u8, u16, bool)>,
) -> BuffRegistry {
    let active_ids: std::collections::HashSet<i64> = content
        .buffs
        .iter()
        .filter(|b| b.available(gw2_build, evtc_build))
        .map(|b| b.id)
        .collect();
    let mut dynamic: Vec<BuffDef> = Vec::new();
    for (label, lists, icon) in [
        ("Unknown Nourishment", &["NormalFoods", "AscendedFood"][..], NOURISHMENT_ICON),
        ("Unknown Enhancement", &["Utilities", "Writs", "SlayingPotions"][..], ENHANCEMENT_ICON),
    ] {
        let cand_cats: std::collections::BTreeSet<u8> = content
            .buffs
            .iter()
            .filter(|b| lists.contains(&b.list.as_str()) && buff_info.contains_key(&(b.id as u32)))
            .filter_map(|b| buff_info.get(&(b.id as u32)))
            .map(|(cat, _, _)| *cat)
            .collect();
        if cand_cats.len() != 1 {
            continue;
        }
        let cat = *cand_cats.iter().next().expect("one");
        let cls = if label.starts_with("Unknown Nourishment") {
            BuffClassification::Nourishment
        } else {
            BuffClassification::Enhancement
        };
        for (&id, &(info_cat, max_stacks, _)) in buff_info {
            if info_cat == cat && !active_ids.contains(&i64::from(id)) {
                // CreateCustomBuff(name, id, link, capacity, classification):
                // name + " " + id;capacity>1 → Stacking else Force。
                let capacity = i64::from(max_stacks);
                let stack_type = if capacity > 1 { "Stacking" } else { "Force" };
                dynamic.push(BuffDef {
                    id: i64::from(id),
                    name: format!("{label} {id}"),
                    classification: cls,
                    stack_type: stack_type.to_string(),
                    capacity,
                    icon: icon.to_string(),
                    sources: vec!["Item".to_string()],
                    min_gw2_build: 0,
                    max_gw2_build: u64::MAX,
                    min_evtc_build: i64::MIN,
                    max_evtc_build: i64::MAX,
                    list: String::new(),
                });
            }
        }
    }
    BuffRegistry::build(&content.buffs, gw2_build, evtc_build, dynamic)
}

// ===== SpecCatalog(gw2ei-model 注入)=====

/// 完整 `GW2APIController.GetSpec` 语义(GW2APIController.cs:114-166):
/// 本地硬编码分支(非玩家/基础职业/HoT)+ SpecList.json 现代分支。
pub struct CatalogSpecs<'a>(pub &'a Content);

impl<'a> gw2ei_model::SpecCatalog for CatalogSpecs<'a> {
    fn spec_of(&self, prof: u32, elite: u32) -> Spec {
        if elite == u32::MAX {
            return if prof & 0xFFFF_0000 == 0xFFFF_0000 {
                Spec::Gadget
            } else {
                Spec::Npc
            };
        }
        match elite {
            0 => match prof {
                1 => Spec::Guardian,
                2 => Spec::Warrior,
                3 => Spec::Engineer,
                4 => Spec::Ranger,
                5 => Spec::Thief,
                6 => Spec::Elementalist,
                7 => Spec::Mesmer,
                8 => Spec::Necromancer,
                9 => Spec::Revenant,
                _ => Spec::Unknown,
            },
            1 => match prof {
                1 => Spec::Dragonhunter,
                2 => Spec::Berserker,
                3 => Spec::Scrapper,
                4 => Spec::Druid,
                5 => Spec::Daredevil,
                6 => Spec::Tempest,
                7 => Spec::Chronomancer,
                8 => Spec::Reaper,
                9 => Spec::Herald,
                _ => Spec::Unknown,
            },
            _ => match self.0.spec_by_id.get(&i64::from(elite)) {
                Some(spec) if spec.elite => spec_from_name(&spec.name),
                Some(spec) => spec_from_name(&spec.profession),
                None => Spec::Unknown,
            },
        }
    }
}

/// C# `ProfToSpec`(spec 名 → 枚举;未知名 → Unknown)。名称与
/// gw2ei-model::Spec::csharp_name 对应。
pub fn spec_from_name(name: &str) -> Spec {
    // 与 spec.rs 枚举逐名对齐(表驱动)。
    for (n, s) in [
        ("Guardian", Spec::Guardian),
        ("Warrior", Spec::Warrior),
        ("Engineer", Spec::Engineer),
        ("Ranger", Spec::Ranger),
        ("Thief", Spec::Thief),
        ("Elementalist", Spec::Elementalist),
        ("Mesmer", Spec::Mesmer),
        ("Necromancer", Spec::Necromancer),
        ("Revenant", Spec::Revenant),
        ("Dragonhunter", Spec::Dragonhunter),
        ("Berserker", Spec::Berserker),
        ("Scrapper", Spec::Scrapper),
        ("Druid", Spec::Druid),
        ("Daredevil", Spec::Daredevil),
        ("Tempest", Spec::Tempest),
        ("Chronomancer", Spec::Chronomancer),
        ("Reaper", Spec::Reaper),
        ("Herald", Spec::Herald),
        ("Firebrand", Spec::Firebrand),
        ("Scourge", Spec::Scourge),
        ("Weaver", Spec::Weaver),
        ("Soulbeast", Spec::Soulbeast),
        ("Deadeye", Spec::Deadeye),
        ("Mirage", Spec::Mirage),
        ("Renegade", Spec::Renegade),
        ("Holosmith", Spec::Holosmith),
        ("Spellbreaker", Spec::Spellbreaker),
        ("Catalyst", Spec::Catalyst),
        ("Bladesworn", Spec::Bladesworn),
        ("Vindicator", Spec::Vindicator),
        ("Willbender", Spec::Willbender),
        ("Virtuoso", Spec::Virtuoso),
        ("Specter", Spec::Specter),
        ("Untamed", Spec::Untamed),
        ("Harbinger", Spec::Harbinger),
        ("Mechanist", Spec::Mechanist),
        ("Troubadour", Spec::Troubadour),
        ("Luminary", Spec::Luminary),
        ("Conduit", Spec::Conduit),
        ("Evoker", Spec::Evoker),
        ("Galeshot", Spec::Galeshot),
        ("Antiquary", Spec::Antiquary),
        ("Paragon", Spec::Paragon),
        ("Amalgam", Spec::Amalgam),
        ("Ritualist", Spec::Ritualist),
    ] {
        if name == n {
            return s;
        }
    }
    Spec::Unknown
}

// ===== 默认加载 =====

/// 从仓库 `rust/content/` 加载全部资产(测试/CLI 默认入口)。相对路径
/// `content_dir` 缺失任一文件 → 显式错误。
#[allow(clippy::field_reassign_with_default)] // 逐资产加载(顺序清晰)
pub fn load_default_content(content_dir: &Path) -> Result<Content, BuildError> {
    let mut c = Content::default();
    c.buffs = load_buff_table(&content_dir.join("buff-table.json"))?;
    // SkillList.json
    {
        let p = content_dir.join("SkillList.json");
        let raw: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&p)
                .map_err(|e| BuildError::Content(format!("read {p:?}: {e}")))?,
        )
        .map_err(|e| BuildError::Content(format!("parse {p:?}: {e}")))?;
        let arr = raw
            .as_array()
            .ok_or_else(|| BuildError::Content("SkillList.json not an array".into()))?;
        for v in arr {
            let id = v["id"].as_i64().unwrap_or(0);
            c.skills_api.insert(
                id,
                ApiSkill {
                    name: v["name"].as_str().unwrap_or("").to_string(),
                    icon: v["icon"].as_str().unwrap_or("").to_string(),
                    ty: v["type"].as_str().map(String::from),
                    weapon_type: v["weapon_type"].as_str().map(String::from),
                    slot: v["slot"].as_str().map(String::from),
                    professions: v["professions"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                        .unwrap_or_default(),
                    categories: v["categories"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()),
                    description: v["description"].as_str().map(String::from),
                    flags: v["flags"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()),
                    dual_wield: v["dual_wield"].as_str().map(String::from),
                    bundle_skills: v["bundle_skills"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect()),
                },
            );
        }
    }
    // skill-overrides.json
    {
        let p = content_dir.join("skill-overrides.json");
        let raw: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&p)
                .map_err(|e| BuildError::Content(format!("read {p:?}: {e}")))?,
        )
        .map_err(|e| BuildError::Content(format!("parse {p:?}: {e}")))?;
        for (k, v) in [("skill_names", &mut c.overrides.names), ("skill_icons", &mut c.overrides.icons)] {
            if let Some(obj) = raw.get(k).and_then(|o| o.as_object()) {
                for (id, val) in obj {
                    if let Ok(id) = id.parse::<i64>()
                        && let Some(s) = val.as_str()
                    {
                        v.insert(id, s.to_string());
                    }
                }
            }
        }
        if let Some(obj) = raw.get("non_critable").and_then(|o| o.as_object()) {
            for (id, val) in obj {
                // C# `ulong build`:正常值为无符号十进制(0/54485/…);解析
                // 器旧版曾把 0 落为 -2147483648 —— 按 i64 位模式还原兼容。
                if let Ok(id) = id.parse::<i64>()
                    && let Some(b) = val.as_u64().or_else(|| val.as_i64().map(|v| v as u64))
                {
                    c.overrides.non_critable.insert(id, b);
                }
            }
        }
    }
    // SpecList.json
    {
        let p = content_dir.join("SpecList.json");
        let raw: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&p)
                .map_err(|e| BuildError::Content(format!("read {p:?}: {e}")))?,
        )
        .map_err(|e| BuildError::Content(format!("parse {p:?}: {e}")))?;
        let arr = raw
            .as_array()
            .ok_or_else(|| BuildError::Content("SpecList.json not an array".into()))?;
        for v in arr {
            c.spec_by_id.insert(
                v["id"].as_i64().unwrap_or(0),
                SpecLine {
                    name: v["name"].as_str().unwrap_or("").to_string(),
                    profession: v["profession"].as_str().unwrap_or("").to_string(),
                    elite: v["elite"].as_bool().unwrap_or(false),
                },
            );
        }
    }
    // MapList.json
    {
        let p = content_dir.join("MapList.json");
        let raw: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&p)
                .map_err(|e| BuildError::Content(format!("read {p:?}: {e}")))?,
        )
        .map_err(|e| BuildError::Content(format!("parse {p:?}: {e}")))?;
        let arr = raw
            .as_array()
            .ok_or_else(|| BuildError::Content("MapList.json not an array".into()))?;
        for v in arr {
            if let (Some(id), Some(name)) = (v["id"].as_i64(), v["name"].as_str()) {
                c.map_names.insert(id, name.to_string());
            }
        }
    }
    // instant-cast finders(extract-instant-casts.py;P3b 合成引擎输入)
    {
        let p = content_dir.join("instant-cast-finders.json");
        c.instant_finders = serde_json::from_str(
            &std::fs::read_to_string(&p)
                .map_err(|e| BuildError::Content(format!("read {p:?}: {e}")))?,
        )
        .map_err(|e| BuildError::Content(format!("parse {p:?}: {e}")))?;
    }

    Ok(c)
}

/// `SkillItem.CanCrit`(SkillItem.cs:128-135):NonCritableSkills 覆盖表,
/// gw2Build < build 才可暴击;表外 true。
pub fn skill_can_crit(content: &Content, id: i64, gw2_build: u64) -> bool {
    match content.overrides.non_critable.get(&id) {
        Some(&build) => gw2_build < build,
        None => true,
    }
}

/// `SkillItem` 默认 icon(SkillImages.MonsterSkill)。
pub fn default_skill_icon() -> &'static str {
    DEFAULT_SKILL_ICON
}
