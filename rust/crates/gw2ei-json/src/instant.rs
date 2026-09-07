//! P3b:Instant-cast finder 合成引擎(EI `EIData/InstantCastFinders` +
//! `ProfHelper.GetProfessionInstantCastFinders` 的最小子集)。
//!
//! 数据源:`rust/content/instant-cast-finders.json`(`rust/tools/
//! extract-instant-casts.py` 从 C# ProfHelpers 抽取;快照 `3b7278f9b`)。
//! 本模块实现:
//! - 运行时事件索引(buff apply/remove/damage/effect/spawn/weapon swap,
//!   按 finder 触发语义)。
//! - finder 解释:构造参数 + fluent 方法链(`methods`) + 结构化 lambda
//!   条件(`LambdaCond`)。表里出现的复杂 lambda 已在抽取脚本翻译成
//!   LambdaCond;翻译不了的条目进 `unresolved`(脚本产出统计,不入表)。
//! - C# 语义锚点:
//!   - `ComputeInstantCastEventsFromFinders`(CombatData.cs:217-248):
//!     Available 的 finder 先打 NotAccurate/Origin 标记,再 ComputeInstantCast。
//!   - `Available`(InstantCastFinder.cs:138-154):enable 条件 + gw2 build
//!     窗口(`< max && >= min`)+ evtc build 窗口。
//!   - 触发骨架(BuffCastFinder.cs:51-73 等):触发事件按 key agent 分组
//!     (Minions → final master),组内时间序遍历,ICD 去重(丢弃也推进
//!     lastTime),命中 emit InstantCastEvent。
//!   - `GetTime`(:117-129):Before/AfterWeaponSwap 时对 ±5ms 内的武器
//!     切换时间吸附。
//! - 事件类型化缺口显式登记:EXTHealing/EXTBarrier/Marker/BuffExtend
//!   依赖的扩展/导弹事件流不在 P3b 面内 → 触发集为空(无输出)并记入
//!   `skipped`(对拍日志可观测,不静默)。
//!
//! EngineerKitFinder(EngineerHelper.cs:16-84)有 per-caster 双游标状态,
//! 手工移植(见 `KitState`)。

use std::collections::{BTreeMap, BTreeSet};

use gw2ei_model::events::CombatEvent;
use gw2ei_model::{AgentId, AgentTable, NO_AGENT, ParsedLog, Spec};
use serde_json::Value;

use crate::content::Content;

pub const SERVER_DELAY: i64 = 10; // ParserHelper.ServerDelayConstant
/// ParserHelper.WeaponSwapDelayConstant(EngineerKit checker 用)。
const WEAPON_SWAP_DELAY: i64 = 75;

/// 每 finder 的 ICD(默认 50ms;InstantCastFinder.DefaultICD)。
const DEFAULT_ICD: i64 = 50;

/// 合成产物:per-caster 时间升序行 + skillData 标记(C# `SkillData` 的
/// NotAccurate/GearProc/TraitProc/UnconditionalProc HashSet)。
#[derive(Debug, Clone, Default)]
pub struct InstantOut {
    /// caster(最终归属 agent) → 合成行(时间升序)。
    pub lines: BTreeMap<AgentId, Vec<InstantLine>>,
    pub not_accurate: BTreeSet<i64>,
    pub gear_proc: BTreeSet<i64>,
    pub trait_proc: BTreeSet<i64>,
    pub unconditional_proc: BTreeSet<i64>,
    /// 因事件面缺失无法评估的 finder(表名登记,可观测不静默)。
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstantLine {
    pub time: i64,
    pub skill_id: i64,
}

/// 全局触发行索引(一次遍历 events)。
struct TriggerIndex<'a> {
    log: &'a ParsedLog,
    resolver: AgentTable,
    /// buff id(signed) → events 索引(apply 事件;含 BuffApply)。
    apply_by_buff: BTreeMap<i64, Vec<usize>>,
    /// remove all 事件 by buff。
    remove_all_by_buff: BTreeMap<i64, Vec<usize>>,
    /// remove single 事件 by buff。
    remove_single_by_buff: BTreeMap<i64, Vec<usize>>,
    /// health damage(直伤/症状/NoDamage)by skill id。
    damage_by_id: BTreeMap<i64, Vec<usize>>,
    /// breakbar damage by skill id。
    breakbar_by_id: BTreeMap<i64, Vec<usize>>,
    /// effect 事件 by effect id(signed)。
    effect_by_id: BTreeMap<i64, Vec<usize>>,
    /// weapon swap 事件 by caster agent。
    swap_by_caster: BTreeMap<AgentId, Vec<usize>>,
    /// 动画 cast(AnimatedCast/Emote/GadgetInteract/BundlePickUp)按 resolved
    /// caster(EngineerKit 的 per-caster 双游标面)。
    animated_by_caster: BTreeMap<AgentId, Vec<usize>>,
    /// 全量 effect 事件数(C# HasEffectData = EffectEvents.Count != 0)。
    effect_count: usize,
    /// spawn 事件(agent 索引;MinionSpawn finder 用,key = 最终 master)。
    spawn_by_master: BTreeMap<AgentId, Vec<(i64, usize)>>,
    /// spawn 事件按 species(agent id == species;MinionSpawn 用)。
    spawn_of_species: BTreeMap<i32, Vec<usize>>,
    /// 动画 cast(AnimatedCast/Emote/GadgetInteract/BundlePickUp)按 skill
    /// (IsCasting/UsingNoAnimatedCastChecker 用)。
    animated_by_id: BTreeMap<i64, Vec<usize>>,
    /// MissileCreate 事件按 skill id(Trigger 面)。
    missile_by_skill: BTreeMap<i64, Vec<usize>>,
    /// MissileCreate 事件总数(C# HasMissileData)。
    missile_count: usize,
}

#[allow(clippy::too_many_lines)]
fn build_index(log: &ParsedLog) -> TriggerIndex<'_> {
    let mut idx = TriggerIndex {
        log,
        resolver: log.agents.clone(),
        apply_by_buff: BTreeMap::new(),
        remove_all_by_buff: BTreeMap::new(),
        remove_single_by_buff: BTreeMap::new(),
        damage_by_id: BTreeMap::new(),
        breakbar_by_id: BTreeMap::new(),
        effect_by_id: BTreeMap::new(),
        swap_by_caster: BTreeMap::new(),
        animated_by_caster: BTreeMap::new(),
        effect_count: 0,
        spawn_by_master: BTreeMap::new(),
        spawn_of_species: BTreeMap::new(),
        animated_by_id: BTreeMap::new(),
        missile_by_skill: BTreeMap::new(),
        missile_count: 0,
    };
    for (i, evt) in log.events.iter().enumerate() {
        match evt {
            CombatEvent::BuffApply(f) => {
                let id = crate::signed_id(f.apply.base.buff_id);
                idx.apply_by_buff.entry(id).or_default().push(i);
            }
            CombatEvent::BuffRemoveAll(f) => {
                let id = crate::signed_id(f.remove.base.buff_id);
                idx.remove_all_by_buff.entry(id).or_default().push(i);
            }
            CombatEvent::BuffRemoveSingle(f) => {
                let id = crate::signed_id(f.remove.base.buff_id);
                idx.remove_single_by_buff.entry(id).or_default().push(i);
            }
            CombatEvent::DirectHealthDamage(f)
            | CombatEvent::NonDirectHealthDamage(f)
            | CombatEvent::NoDamageHealthDamage(f) => {
                let id = crate::signed_id(f.skill.skill_id);
                idx.damage_by_id.entry(id).or_default().push(i);
            }
            CombatEvent::BreakbarDamage(f) | CombatEvent::BreakbarRecovery(f) => {
                let id = crate::signed_id(f.skill.skill_id);
                idx.breakbar_by_id.entry(id).or_default().push(i);
            }
            CombatEvent::Effect(f) => {
                let id = crate::signed_id(f.effect_id);
                idx.effect_count += 1;
                idx.effect_by_id.entry(id).or_default().push(i);
            }
            CombatEvent::AnimatedCast(f) | CombatEvent::GadgetInteract(f) => {
                let id = crate::signed_id(f.skill_id);
                idx.animated_by_id.entry(id).or_default().push(i);
                let time = f.time;
                let caster = resolve(&mut idx.resolver, f.caster, time);
                if caster != NO_AGENT {
                    idx.animated_by_caster.entry(caster).or_default().push(i);
                }
            }
            CombatEvent::Emote(f) => {
                let id = crate::signed_id(f.base.skill_id);
                idx.animated_by_id.entry(id).or_default().push(i);
                let time = f.base.time;
                let caster = resolve(&mut idx.resolver, f.base.caster, time);
                if caster != NO_AGENT {
                    idx.animated_by_caster.entry(caster).or_default().push(i);
                }
            }
            CombatEvent::BundlePickUp(f) => {
                let id = crate::signed_id(f.base.skill_id);
                idx.animated_by_id.entry(id).or_default().push(i);
                // BundlePickUp.Caster = unknown(0)(model 构造语义)→ 不入
                // per-caster 表(C# BundlePickUpEvent Caster=unknown 同)。
            }
            CombatEvent::Missile(f) => {
                let id = crate::signed_id(f.skill_id);
                idx.missile_count += 1;
                idx.missile_by_skill.entry(id).or_default().push(i);
            }
            CombatEvent::WeaponSwap(f) => {
                let time = f.time;
                let caster = resolve(&mut idx.resolver, f.caster, time);
                if caster != NO_AGENT {
                    idx.swap_by_caster.entry(caster).or_default().push(i);
                }
            }
            CombatEvent::Spawn(f) => {
                let time = f.time;
                let a = resolve(&mut idx.resolver, f.src, time);
                if a != NO_AGENT
                    && let Some(agent) = idx.log.agents.slot(a)
                {
                    // MinionSpawn:按 species 收(仅带 master 的 minion
                    // 才能产生 instant —— C# GetStableSpeciesByID 后
                    // Where(m => m.Master != null))。
                    let master = idx.log.agents.final_master(a);
                    if master != a {
                        idx.spawn_by_master.entry(master).or_default().push((time, i));
                    }
                    if !agent.is_unknown {
                        idx.spawn_of_species.entry(agent.id).or_default().push(i);
                    }
                }
            }
            _ => {}
        }
    }
    idx
}

fn resolve(table: &mut AgentTable, addr: u64, time: i64) -> AgentId {
    if addr == 0 {
        return NO_AGENT;
    }
    table.resolve_agent(addr, time)
}

/// agent id → 事件行索引列表(时间序;events 全局时间序)。
fn rows_of(map: &BTreeMap<i64, Vec<usize>>, id: i64) -> &[usize] {
    map.get(&id).map(|v| v.as_slice()).unwrap_or(&[])
}

// ===== finder 结构化解释 =====

/// 触发参数(args[1])的已解析值。
#[derive(Debug, Clone)]
enum MArg {
    Int(i64),
    Guid(String),
    Ids(Vec<i64>),
    /// 表内 GUID 常量缺失 / 其它未结构化形态。
    Other,
}

fn parse_marg(v: &Value) -> Option<MArg> {
    match v {
        Value::Number(n) => n.as_i64().map(MArg::Int),
        Value::Object(o) => {
            if let Some(g) = o.get("guid").and_then(|x| x.as_str()) {
                return Some(MArg::Guid(g.to_string()));
            }
            if let Some(arr) = o.get("idset").and_then(|x| x.as_array()) {
                return Some(MArg::Ids(arr.iter().filter_map(|x| x.as_i64()).collect()));
            }
            // {"guid": null} 或 {"unresolved": …} → Other(不触发)
            Some(MArg::Other)
        }
        Value::Array(a) => {
            let ints: Vec<i64> = a.iter().filter_map(|x| x.as_i64()).collect();
            if ints.len() == a.len() {
                Some(MArg::Ids(ints))
            } else {
                Some(MArg::Other)
            }
        }
        _ => Some(MArg::Other),
    }
}

/// finder 运行时配置(表条目解释后)。
struct Finder {
    kind: String,
    /// 产出 skill id(args[0])。
    skill_id: i64,
    /// 触发 id(args[1];buff/damage/effect-guid/species/idset)。
    arg1: Option<MArg>,
    /// args[2..](MinionSpawn idset 等已并入 arg1)。
    not_accurate: bool,
    origin: Origin,
    icd: i64,
    time_offset: i64,
    before_swap: bool,
    after_swap: bool,
    minions: bool,
    with_minions_override: bool,
    build_min: u64,
    build_max: u64,
    evtc_min: i64,
    evtc_max: i64,
    enable_has_effect: bool,
    disable_has_effect: bool,
    disable_has_missile: bool,
    /// 结构化条件(LambdaCond / 机械方法)。
    conds: Vec<Cond>,
}

/// jarr(serde_json::json!([..])) 的机械替换。
fn jarr(v: serde_json::Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

#[derive(Debug, Clone)]
struct Cond {
    name: String,
    args: Vec<Value>,
    neg: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Skill,
    Trait,
    Gear,
    Unconditional,
}

/// 表条目 JSON → Finder(把 fluent methods 应用到默认值)。
#[allow(clippy::too_many_lines)]
fn decode_finder(entry: &Value, label: &str) -> Option<Finder> {
    let kind = entry["kind"].as_str()?.to_string();
    let args = entry["args"].as_array()?;
    let mut f = Finder {
        skill_id: args.first().and_then(|v| v.as_i64()).unwrap_or(0),
        arg1: args.get(1).and_then(parse_marg),
        kind,
        not_accurate: false,
        origin: Origin::Skill,
        icd: DEFAULT_ICD,
        time_offset: 0,
        before_swap: false,
        after_swap: false,
        minions: false,
        with_minions_override: false,
        build_min: 0,
        build_max: u64::MAX,
        evtc_min: i64::MIN,
        evtc_max: i64::MAX,
        enable_has_effect: false,
        disable_has_effect: false,
        disable_has_missile: false,
        conds: Vec::new(),
    };
    // kind 内置(C# 构造器内调用,表 JSON 不重复)
    match f.kind.as_str() {
        "BuffGainCastFinder" | "BuffGiveCastFinder" => {
            // UsingChecker(!Initial)(BuffGainCastFinder.cs:14-16 等)
            f.conds.push(Cond {
                name: "not_initial".into(),
                args: vec![],
                neg: false,
            });
        }
        "MinionCommandCastFinder" => {
            f.minions = true; // MinionCommandCastFinder ctor(Minions = true)
            f.conds.push(Cond {
                name: "not_initial".into(),
                args: vec![],
                neg: false,
            });
        }
        "DamageCastFinder" | "BreakbarDamageCastFinder" => f.not_accurate = true,
        "EffectCastFinder" | "EffectCastFinderByDst" => {
            f.not_accurate = true;
            f.enable_has_effect = true; // UsingEnable(HasEffectData)
        }
        "EngineerKit" => {
            // WeaponSwapCastFinder 子类:BeforeSwap 内置 + NotAccurate
            f.before_swap = true;
            f.not_accurate = true;
        }
        "EXTHealingCastFinder" | "EXTBarrierCastFinder" | "MissileCastFinder"
        | "MarkerCastFinder" | "WeaponSwapCastFinder" | "BuffExtendCastFinder" => {
            f.not_accurate = false;
        }
        _ => {}
    }
    // fluent 方法
    for m in entry["methods"].as_array().into_iter().flatten() {
        let name = m["name"].as_str().unwrap_or("");
        match name {
            "UsingICD" => f.icd = marg_int(m, 0).unwrap_or(DEFAULT_ICD),
            "UsingOrigin" => {
                f.origin = match m["args"][0].as_str().unwrap_or("") {
                    "Trait" => Origin::Trait,
                    "Gear" => Origin::Gear,
                    "Unconditional" => Origin::Unconditional,
                    _ => Origin::Skill,
                };
            }
            "UsingNotAccurate" => f.not_accurate = true,
            "UsingTimeOffset" => f.time_offset = marg_int(m, 0).unwrap_or(0),
            "UsingBeforeWeaponSwap" => {
                f.before_swap = true;
                f.after_swap = false;
            }
            "UsingAfterWeaponSwap" => {
                f.after_swap = true;
                f.before_swap = false;
            }
            "UsingEnable" => {
                // lambda enable checker —— 表内均为 HasEffectData 形态的
                // 反义(UsingDisableWithEffectData);原文无法解释时登记。
                f.disable_has_effect = true;
            }
            "UsingDisableWithEffectData" => f.disable_has_effect = true,
            "UsingDisableWithMissileData" => f.disable_has_missile = true,
            "WithBuilds" => {
                f.build_min = marg_u64(m, 0).unwrap_or(0);
                f.build_max = marg_u64(m, 1).unwrap_or(u64::MAX);
            }
            "WithEvtcBuilds" => {
                f.evtc_min = marg_int(m, 0).unwrap_or(i64::MIN);
                f.evtc_max = marg_int(m, 1).unwrap_or(i64::MAX);
            }
            "WithMinions" => {
                f.minions = true;
                f.with_minions_override = true;
            }
            "LambdaCond" => {
                if let Some(c) = m["cond"].as_object() {
                    f.conds.push(Cond {
                        name: c["cond"].as_str().unwrap_or("").to_string(),
                        args: c["args"].as_array().cloned().unwrap_or_default(),
                        neg: c["neg"].as_bool().unwrap_or(false),
                    });
                }
            }
            // 机械 spec checker → Cond
            "UsingByBaseSpecChecker" | "UsingToBaseSpecChecker" | "UsingSrcBaseSpecChecker"
            | "UsingDstBaseSpecChecker" | "UsingBySpecChecker" | "UsingToSpecChecker"
            | "UsingSrcSpecChecker" | "UsingDstSpecChecker" => {
                if let Some(spec) = m["args"][0].get("spec").and_then(|s| s.as_str()) {
                    let side = if name.contains("To") || name.contains("Dst") {
                        "to"
                    } else if name.contains("By") {
                        "by"
                    } else {
                        "src"
                    };
                    let base = name.contains("Base");
                    f.conds.push(Cond {
                        name: "agent_spec".into(),
                        args: jarr(serde_json::json!([side, base, spec])),
                        neg: false,
                    });
                }
            }
            "UsingByNotSpecChecker" | "UsingToNotSpecChecker" | "UsingSrcNotSpecChecker"
            | "UsingDstNotSpecChecker" => {
                if let Some(spec) = m["args"][0]
                    .get("spec")
                    .and_then(|s| s.as_str())
                {
                    let side = if name.contains("To") || name.contains("Dst") {
                        "to"
                    } else if name.contains("By") {
                        "by"
                    } else {
                        "src"
                    };
                    let base = name.contains("Base");
                    f.conds.push(Cond {
                        name: "agent_spec".into(),
                        args: jarr(serde_json::json!([side, base, spec])),
                        neg: true,
                    });
                }
            }
            "UsingDurationChecker" => {
                let dur = marg_int(m, 0).unwrap_or(0);
                let eps = marg_int(m, 1).unwrap_or(SERVER_DELAY);
                f.conds.push(Cond {
                    name: "duration_approx".into(),
                    args: jarr(serde_json::json!([dur, eps])),
                    neg: false,
                });
            }
            "UsingIsAroundDstChecker" | "UsingNotIsAroundDstChecker" => {
                f.conds.push(Cond {
                    name: "is_around_dst".into(),
                    args: vec![],
                    neg: name.contains("Not"),
                });
            }
            "UsingDstSpecsChecker" | "UsingDstNotSpecsChecker" | "UsingSrcSpecsChecker"
            | "UsingSrcNotSpecsChecker" | "UsingToSpecsChecker" | "UsingToNotSpecsChecker"
            | "UsingBySpecsChecker" | "UsingByNotSpecsChecker" => {
                // 参数为 [Spec.X, ...] 数组(JSON idset 化失败时走 Flag)——
                // 抽取脚本对 Spec 数组原样输出为对象数组;这里取全部 spec
                let side = if name.contains("Dst") {
                    "to"
                } else if name.contains("Src") {
                    "src"
                } else if name.contains("By") {
                    "by"
                } else {
                    "to"
                };
                let base = name.contains("Base");
                let neg = name.contains("Not");
                let specs: Vec<String> = m["args"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|a| a.get("spec").and_then(|s| s.as_str()).map(String::from))
                    .collect();
                if !specs.is_empty() {
                    f.conds.push(Cond {
                        name: "agent_spec_multi".into(),
                        args: jarr(serde_json::json!([side, base, neg, specs])),
                        neg: false,
                    });
                }
            }
            "UsingAgentRedirectionIfUnknown" => {}
            "UsingSrcNotBaseSpecChecker" => {
                if let Some(spec) = m["args"][0].get("spec").and_then(|x| x.as_str()) {
                    f.conds.push(Cond {
                        name: "agent_spec".into(),
                        args: jarr(serde_json::json!(["src", true, spec])),
                        neg: true,
                    });
                }
            }
            // Effect 系 secondary/no-animated checker(参数:{guid}|{t}|数字)
            "UsingNoAnimatedCastChecker" => {
                let skill = m["args"][0].as_i64().unwrap_or(0);
                let t_off = m["args"].get(1).and_then(|a| a.get("t")).and_then(|x| x.as_i64()).unwrap_or(0);
                let eps = m["args"].get(2).and_then(|x| x.as_i64()).unwrap_or(SERVER_DELAY);
                f.conds.push(Cond {
                    name: "no_animated_cast".into(),
                    args: jarr(serde_json::json!([skill, t_off, eps])),
                    neg: false,
                });
            }
            "UsingSecondaryEffectSameSrcChecker" => {
                push_secondary(&mut f, m, "secondary_effect_same_src", false);
            }
            "UsingSecondaryEffectInvertedSrcChecker" => {
                push_secondary(&mut f, m, "secondary_effect_inverted_src", false);
            }
            "UsingNoSecondaryEffectSameSrcCheckerOnSamePosition" => {
                push_secondary(&mut f, m, "secondary_effect_same_src", true);
            }
            // 无法直接解释的方法(罕见;登记不阻断)
            other => {
                eprintln!(
                    "instant: finder {label} unhandled method {other} ({} {})",
                    f.kind, f.skill_id
                );
            }
        }
    }
    Some(f)
}

/// SecondaryEffect 系 checker → cond(args: [guid, t_off, eps] 或带位置)。
fn push_secondary(f: &mut Finder, m: &Value, cond: &str, no: bool) {
    let guid = m["args"][0]
        .get("guid")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let t_off = m["args"]
        .get(1)
        .and_then(|a| a.get("t"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let eps = m["args"]
        .get(2)
        .and_then(|x| x.as_i64())
        .unwrap_or(SERVER_DELAY);
    let args = jarr(serde_json::json!([guid, t_off, eps]));
    let name = if no && cond.starts_with("secondary_effect_same_src") {
        "no_secondary_same_src_same_position".to_string()
    } else {
        cond.to_string()
    };
    f.conds.push(Cond { name, args, neg: false });
}

fn marg_int(m: &Value, i: usize) -> Option<i64> {
    m["args"].as_array()?.get(i)?.as_i64()
}
fn marg_u64(m: &Value, i: usize) -> Option<u64> {
    m["args"].as_array()?.get(i)?.as_i64().map(|x| x as u64)
}

/// 表条目 arg[1] 取值帮助:Int / Ids。
fn arg1_int(f: &Finder) -> i64 {
    match &f.arg1 {
        Some(MArg::Int(v)) => *v,
        _ => 0,
    }
}
fn arg1_guid(f: &Finder) -> Option<&str> {
    match &f.arg1 {
        Some(MArg::Guid(g)) => Some(g),
        _ => None,
    }
}

/// `Available`(InstantCastFinder.cs:138-154)+ kind 内置 enable。
fn available(f: &Finder, gw2_build: u64, evtc_build: i64, idx: &TriggerIndex<'_>) -> bool {
    if f.disable_has_effect && idx.effect_count != 0 {
        return false;
    }
    // UsingDisableWithMissileData(C# combatData.HasMissileData)
    if f.disable_has_missile && idx.missile_count != 0 {
        return false;
    }
    if !(gw2_build < f.build_max && gw2_build >= f.build_min) {
        return false;
    }
    if !(evtc_build < f.evtc_max && evtc_build >= f.evtc_min) {
        return false;
    }
    if f.enable_has_effect && idx.effect_count == 0 {
        return false;
    }
    true
}

// ===== 事件触发视图 =====

/// finder 触发事件的统一视图(kind 分派的字段面)。
struct View {
    time: i64,
    /// key agent 相关地址(未解析)。buff: by/to;effect: src/dst;
    /// damage: from/to;spawn: src。
    src_addr: u64,
    dst_addr: u64,
    by_addr: u64,
    to_addr: u64,
    /// BuffApply 额外面。
    initial: bool,
    applied_duration: i64,
    /// BuffRemoveAll 面。
    removed_duration: i64,
    /// BuffRemoveSingle 面。
    buff_instance: u32,
    /// Effect 面。
    around_dst: bool,
    duration: i64,
    /// EffectEnd 配对(end_time;None = 尚无 end)。
    end_time: Option<i64>,
    /// Effect 位置(ground 有效;Agent/CBTS around-dst 时无效)。
    position: (f32, f32, f32),
}

/// 触发事件列表(按 kind 从索引取)。
#[allow(clippy::too_many_lines)]
fn trigger_events<'a>(
    idx: &'a TriggerIndex<'a>,
    f: &Finder,
) -> Option<(&'static str, &'a [usize])> {
    match f.kind.as_str() {
        "BuffGainCastFinder" | "BuffGiveCastFinder" => {
            Some(("apply", rows_of(&idx.apply_by_buff, arg1_int(f))))
        }
        // MinionCommandCastFinder:BuffGainCastFinder(skill, MinionCommandBuff)
        // 子类(MinionCommandCastFinder.cs:12-22)—— 触发 buff 恒为
        // MinionCommandBuff(59536),JSON 第二参是 species(条件用)。
        "MinionCommandCastFinder" => {
            Some(("apply", rows_of(&idx.apply_by_buff, 59_536)))
        }
        "BuffLossCastFinder" => Some(("remove_all", rows_of(&idx.remove_all_by_buff, arg1_int(f)))),
        "BuffExtendCastFinder" => None,
        "DamageCastFinder" => Some(("damage", rows_of(&idx.damage_by_id, arg1_int(f)))),
        "BreakbarDamageCastFinder" => {
            Some(("breakbar", rows_of(&idx.breakbar_by_id, arg1_int(f))))
        }
        "EffectCastFinder" | "EffectCastFinderByDst" => {
            // GUID → effect id(C# GetEffectGUIDEventByGUID → Dummy 语义)
            let Some(guid) = arg1_guid(f) else {
                return Some(("effect", &[]));
            };
            let effect_id = idx
                .log
                .metadata
                .effect_guid_by_effect_id
                .iter()
                .find(|(_, info)| info.guid_hex == guid)
                .map(|(id, _)| *id);
            let Some(eid) = effect_id else { return Some(("effect", &[])) };
            Some(("effect", rows_of(&idx.effect_by_id, eid)))
        }
        "MinionSpawnCastFinder" => Some(("spawn", &[])),
        "MissileCastFinder" => Some(("missile", rows_of(&idx.missile_by_skill, arg1_int(f)))),
        "MinionCastCastFinder" | "MarkerCastFinder" | "WeaponSwapCastFinder"
        | "EXTHealingCastFinder" | "EXTBarrierCastFinder" | "EngineerKit" => None,
        _ => None,
    }
}

fn view_of(log: &ParsedLog, ev: usize) -> View {
    match &log.events[ev] {
        CombatEvent::BuffApply(f) => View {
            time: f.apply.base.time,
            src_addr: 0,
            dst_addr: 0,
            by_addr: f.apply.base.by,
            to_addr: f.apply.base.to,
            initial: f.initial,
            applied_duration: i64::from(f.applied_duration),
            removed_duration: 0,
            buff_instance: f.apply.buff_instance,
            around_dst: false,
            duration: 0,
            end_time: None,
            position: (0.0, 0.0, 0.0),
        },
        CombatEvent::BuffRemoveAll(f) => View {
            time: f.remove.base.time,
            src_addr: 0,
            dst_addr: 0,
            by_addr: f.remove.base.by,
            to_addr: f.remove.base.to,
            initial: false,
            applied_duration: 0,
            removed_duration: i64::from(f.remove.removed_duration),
            buff_instance: 0,
            around_dst: false,
            duration: 0,
            end_time: None,
            position: (0.0, 0.0, 0.0),
        },
        CombatEvent::BuffRemoveSingle(f) => View {
            time: f.remove.base.time,
            src_addr: 0,
            dst_addr: 0,
            by_addr: f.remove.base.by,
            to_addr: f.remove.base.to,
            initial: false,
            applied_duration: 0,
            removed_duration: i64::from(f.remove.removed_duration),
            buff_instance: f.buff_instance,
            around_dst: false,
            duration: 0,
            end_time: None,
            position: (0.0, 0.0, 0.0),
        },
        CombatEvent::DirectHealthDamage(f)
        | CombatEvent::NonDirectHealthDamage(f)
        | CombatEvent::NoDamageHealthDamage(f) => View {
            time: f.skill.time,
            src_addr: f.skill.from,
            dst_addr: f.skill.to,
            by_addr: 0,
            to_addr: 0,
            initial: false,
            applied_duration: 0,
            removed_duration: 0,
            buff_instance: 0,
            around_dst: false,
            duration: 0,
            end_time: None,
            position: (0.0, 0.0, 0.0),
        },
        CombatEvent::BreakbarDamage(f) | CombatEvent::BreakbarRecovery(f) => View {
            time: f.skill.time,
            src_addr: f.skill.from,
            dst_addr: f.skill.to,
            by_addr: 0,
            to_addr: 0,
            initial: false,
            applied_duration: 0,
            removed_duration: 0,
            buff_instance: 0,
            around_dst: false,
            duration: 0,
            end_time: None,
            position: (0.0, 0.0, 0.0),
        },
        CombatEvent::Effect(f) => View {
            time: f.time,
            src_addr: f.src,
            dst_addr: f.dst,
            by_addr: 0,
            to_addr: 0,
            initial: false,
            applied_duration: 0,
            removed_duration: 0,
            buff_instance: 0,
            around_dst: f.dst != 0,
            duration: f.duration,
            end_time: f.end_time,
            position: f.position,
        },
        CombatEvent::Missile(f) => View {
            time: f.time,
            src_addr: f.src,
            dst_addr: 0,
            by_addr: 0,
            to_addr: 0,
            initial: false,
            applied_duration: 0,
            removed_duration: 0,
            buff_instance: 0,
            around_dst: false,
            duration: 0,
            end_time: None,
            position: (0.0, 0.0, 0.0),
        },
        // 动画 cast 系(IsCasting/NoAnimatedCastChecker 窗口面;EndTime =
        // Time + ActualDuration,CastEvent.cs:29)。BundlePickUp 的 Caster
        // 置 unknown(0)(model 构造语义),与 C# 一致。
        CombatEvent::AnimatedCast(f) | CombatEvent::GadgetInteract(f) => View {
            time: f.time,
            src_addr: f.caster,
            dst_addr: f.effect_target,
            by_addr: 0,
            to_addr: 0,
            initial: false,
            applied_duration: 0,
            removed_duration: 0,
            buff_instance: 0,
            around_dst: false,
            duration: i64::from(f.actual_duration),
            end_time: None,
            position: (0.0, 0.0, 0.0),
        },
        CombatEvent::Emote(f) => View {
            time: f.base.time,
            src_addr: f.base.caster,
            dst_addr: f.base.effect_target,
            by_addr: 0,
            to_addr: 0,
            initial: false,
            applied_duration: 0,
            removed_duration: 0,
            buff_instance: 0,
            around_dst: false,
            duration: i64::from(f.base.actual_duration),
            end_time: None,
            position: (0.0, 0.0, 0.0),
        },
        CombatEvent::BundlePickUp(f) => View {
            time: f.base.time,
            src_addr: 0, // BundlePickUp.Caster = unknown(C# 构造)
            dst_addr: f.base.effect_target,
            by_addr: 0,
            to_addr: 0,
            initial: false,
            applied_duration: 0,
            removed_duration: 0,
            buff_instance: 0,
            around_dst: false,
            duration: i64::from(f.base.actual_duration),
            end_time: None,
            position: (0.0, 0.0, 0.0),
        },
        // Spawn/Despawn(MinionSpawn 触发面;MinionSpawnCastFinder/
        // has_spawned_minion 消费 src/time)。
        CombatEvent::Spawn(f) | CombatEvent::Despawn(f) => View {
            time: f.time,
            src_addr: f.src,
            dst_addr: 0,
            by_addr: 0,
            to_addr: 0,
            initial: false,
            applied_duration: 0,
            removed_duration: 0,
            buff_instance: 0,
            around_dst: false,
            duration: 0,
            end_time: None,
            position: (0.0, 0.0, 0.0),
        },
        _ => View {
            time: 0,
            src_addr: 0,
            dst_addr: 0,
            by_addr: 0,
            to_addr: 0,
            initial: false,
            applied_duration: 0,
            removed_duration: 0,
            buff_instance: 0,
            around_dst: false,
            duration: 0,
            end_time: None,
            position: (0.0, 0.0, 0.0),
        },
    }
}

/// kind 的 key agent 地址(C# GetKeyAgent)。
fn key_addr_of(f: &Finder, v: &View) -> u64 {
    match f.kind.as_str() {
        "BuffGainCastFinder" | "BuffLossCastFinder" | "MinionCommandCastFinder"
        | "BuffExtendCastFinder" => v.to_addr,
        "BuffGiveCastFinder" => v.by_addr,
        "DamageCastFinder" | "BreakbarDamageCastFinder" => v.src_addr,
        "EffectCastFinder" => v.src_addr,
        "EffectCastFinderByDst" => v.dst_addr,
        _ => v.src_addr,
    }
}

/// key agent 分组键(C# GetCasterAgent:minions → final master)。
fn group_key(log: &ParsedLog, f: &Finder, key_agent: AgentId) -> AgentId {
    if f.minions {
        log.agents.final_master(key_agent)
    } else {
        key_agent
    }
}

// ===== 条件检查 =====

fn check_conds(
    log: &ParsedLog,
    idx: &TriggerIndex<'_>,
    f: &Finder,
    v: &View,
    key_agent: AgentId,
    caster: AgentId,
    resolver: &mut AgentTable,
) -> bool {
    for c in &f.conds {
        let ok = check_cond(log, idx, c, v, key_agent, caster, resolver);
        if !ok {
            return false;
        }
    }
    true
}

fn agent_spec(log: &ParsedLog, a: AgentId) -> (Spec, Spec) {
    match log.agents.slot(a) {
        Some(x) => (x.spec, x.base_spec),
        None => (Spec::Unknown, Spec::Unknown),
    }
}

fn spec_from_name(name: &str) -> Spec {
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

fn same_identity(log: &ParsedLog, a: AgentId, b: AgentId) -> bool {
    log.agents.same_identity(a, b)
}

#[allow(clippy::too_many_lines)]
fn side_addr(v: &View, side: &str) -> u64 {
    match side {
        "src" => v.src_addr,
        "dst" => v.dst_addr,
        "by" => {
            if v.by_addr != 0 {
                v.by_addr
            } else {
                v.src_addr
            }
        }
        "to" => {
            if v.to_addr != 0 {
                v.to_addr
            } else {
                v.dst_addr
            }
        },
        _ => v.src_addr,
    }
}

/// FindRelatedEvents(list, time, eps)(CombatDataHelpers.cs:8-10):时间对称
/// 窗口 |evt.Time - time| < eps。
fn related_hit(
    log: &ParsedLog,
    rows: &[usize],
    t: i64,
    eps: i64,
    mut cond: impl FnMut(&View) -> bool,
) -> bool {
    rows.iter().any(|&ev| {
        let v = view_of(log, ev);
        (v.time - t).abs() < eps && cond(&v)
    })
}

#[allow(clippy::too_many_lines)]
fn check_cond(
    log: &ParsedLog,
    idx: &TriggerIndex<'_>,
    c: &Cond,
    v: &View,
    key_agent: AgentId,
    _caster: AgentId,
    resolver: &mut AgentTable,
) -> bool {
    let res = match c.name.as_str() {
        "not_initial" => !v.initial,
        "duration_approx" => {
            let want = c.args.first().and_then(|x| x.as_i64()).unwrap_or(0);
            let eps = c.args.get(1).and_then(|x| x.as_i64()).unwrap_or(SERVER_DELAY);
            // buff 行用 applied_duration;effect 行用 duration(View 中两者
            // 互斥:effect 构造 applied=0)。
            let d = if v.applied_duration != 0 || v.duration == 0 {
                v.applied_duration
            } else {
                v.duration
            };
            (d - want).abs() < eps
        }
        "agent_spec" => {
            let side = c.args[0].as_str().unwrap_or("to");
            let base = c.args[1].as_bool().unwrap_or(false);
            let name = c.args[2].as_str().unwrap_or("");
            let addr = side_addr(v, side);
            if addr == 0 {
                return false;
            }
            let agent = resolve(resolver, addr, v.time);
            let (spec, base_spec) = agent_spec(log, agent);
            let want = spec_from_name(name);
            if base {
                base_spec == want
            } else {
                spec == want
            }
        }
        "agent_spec_multi" => {
            let side = c.args[0].as_str().unwrap_or("to");
            let base = c.args[1].as_bool().unwrap_or(false);
            let neg = c.args[2].as_bool().unwrap_or(false);
            let names: Vec<&str> = c.args[3]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str())
                .collect();
            let addr = side_addr(v, side);
            if addr == 0 {
                return neg;
            }
            let agent = resolve(resolver, addr, v.time);
            let (spec, base_spec) = agent_spec(log, agent);
            let m = if base {
                names.iter().any(|n| base_spec == spec_from_name(n))
            } else {
                names.iter().any(|n| spec == spec_from_name(n))
            };
            if neg {
                !m
            } else {
                m
            }
        }
        "is_around_dst" => v.around_dst,
        "has_gained_buff" => {
            // HasGainedBuff(buff, agent, time[, appliedDuration, source])
            let buff = c.args.first().and_then(|x| x.as_i64()).unwrap_or(0);
            let side = c
                .args
                .get(1)
                .and_then(|a| a.get("agent"))
                .and_then(|x| x.as_str())
                .unwrap_or("to");
            let want_dur = c.args.get(3).and_then(|x| x.as_i64());
            let src_side = c
                .args
                .get(4)
                .and_then(|a| a.get("agent"))
                .and_then(|x| x.as_str());
            let t = v.time
                + c.args
                    .get(2)
                    .and_then(|a| a.get("t"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
            let addr = side_addr(v, side);
            if addr == 0 {
                return false;
            }
            let agent = resolve(resolver, addr, v.time);
            let rows = rows_of(&idx.apply_by_buff, buff);
            related_hit(log, rows, t, SERVER_DELAY, |ev2| {
                if ev2.to_addr == 0 {
                    return false;
                }
                let to = resolve(resolver, ev2.to_addr, ev2.time);
                let mut ok = same_identity(log, to, agent);
                if ok && want_dur.is_some() {
                    ok = (ev2.applied_duration - want_dur.unwrap_or(0)).abs() < SERVER_DELAY;
                }
                if ok && src_side.is_some() {
                    let saddr = side_addr(ev2, src_side.unwrap_or("by"));
                    if saddr != 0 {
                        let by = resolve(resolver, saddr, ev2.time);
                        ok = same_identity(log, by, agent);
                    }
                }
                ok
            })
        }
        "has_lost_buff" => {
            let buff = c.args.first().and_then(|x| x.as_i64()).unwrap_or(0);
            let side = c
                .args
                .get(1)
                .and_then(|a| a.get("agent"))
                .and_then(|x| x.as_str())
                .unwrap_or("to");
            let t = v.time
                + c.args
                    .get(2)
                    .and_then(|a| a.get("t"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
            let addr = side_addr(v, side);
            if addr == 0 {
                return false;
            }
            let agent = resolve(resolver, addr, v.time);
            let rows = rows_of(&idx.remove_all_by_buff, buff);
            related_hit(log, rows, t, SERVER_DELAY, |ev2| {
                if ev2.to_addr == 0 {
                    return false;
                }
                let to = resolve(resolver, ev2.to_addr, ev2.time);
                same_identity(log, to, agent)
            })
        }
        "has_lost_buff_stack" => {
            // HasLostBuffStack(buff, agent, time, eps):任一 remove
            // (single/all/manual)事件 ±eps(默认 ServerDelayConstant ——
            // C# CombatDataHelpers.cs:52 签名默认)。
            let buff = c.args.first().and_then(|x| x.as_i64()).unwrap_or(0);
            let side = c
                .args
                .get(1)
                .and_then(|a| a.get("agent"))
                .and_then(|x| x.as_str())
                .unwrap_or("to");
            let eps = c
                .args
                .get(3)
                .and_then(|x| x.as_i64())
                .unwrap_or(SERVER_DELAY);
            let t = v.time
                + c.args
                    .get(2)
                    .and_then(|a| a.get("t"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
            let addr = side_addr(v, side);
            if addr == 0 {
                return false;
            }
            let agent = resolve(resolver, addr, v.time);
            let mut found = false;
            'outer: for rows in [
                rows_of(&idx.remove_single_by_buff, buff),
                rows_of(&idx.remove_all_by_buff, buff),
            ] {
                for &ev in rows {
                    let ev2 = view_of(log, ev);
                    if (ev2.time - t).abs() < eps {
                        let to = resolve(resolver, ev2.to_addr, ev2.time);
                        if same_identity(log, to, agent) {
                            found = true;
                            break 'outer;
                        }
                    }
                }
            }
            found
        }
        "has_related_effect_dst" => {
            let guid = c.args[0].as_str().unwrap_or("");
            let side = c
                .args
                .get(1)
                .and_then(|a| a.get("agent"))
                .and_then(|x| x.as_str())
                .unwrap_or("dst");
            let t = v.time
                + c.args
                    .get(2)
                    .and_then(|a| a.get("t"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
            let eid = log
                .metadata
                .effect_guid_by_effect_id
                .iter()
                .find(|(_, info)| info.guid_hex == guid)
                .map(|(id, _)| *id);
            let Some(eid) = eid else { return false };
            let rows = rows_of(&idx.effect_by_id, eid);
            let addr = side_addr(v, side);
            if addr == 0 {
                return false;
            }
            let agent = resolve(resolver, addr, v.time);
            related_hit(log, rows, t, SERVER_DELAY, |ev2| {
                if ev2.dst_addr == 0 {
                    return false;
                }
                let dst = resolve(resolver, ev2.dst_addr, ev2.time);
                same_identity(log, dst, agent)
            })
        }
        "has_related_hit" => {
            let skill = c.args[0].as_i64().unwrap_or(0);
            let side = c
                .args
                .get(1)
                .and_then(|a| a.get("agent"))
                .and_then(|x| x.as_str())
                .unwrap_or("src");
            let t = v.time
                + c.args
                    .get(2)
                    .and_then(|a| a.get("t"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
            let rows = rows_of(&idx.damage_by_id, skill);
            let addr = side_addr(v, side);
            if addr == 0 {
                return false;
            }
            let agent = resolve(resolver, addr, v.time);
            related_hit(log, rows, t, SERVER_DELAY, |ev2| {
                if ev2.src_addr == 0 {
                    return false;
                }
                let from = resolve(resolver, ev2.src_addr, ev2.time);
                same_identity(log, from, agent)
            })
        }
        "is_species" => {
            let side = c.args[0].as_str().unwrap_or("src");
            let species = c.args[1].as_i64().unwrap_or(0);
            let addr = side_addr(v, side);
            if addr == 0 {
                return false;
            }
            let agent = resolve(resolver, addr, v.time);
            agent != NO_AGENT
                && log.agents.slot(agent).is_some_and(|a| i64::from(a.id) == species)
        }
        "is_around_dst_species" => {
            let species = c.args[0].as_i64().unwrap_or(0);
            if !v.around_dst || v.dst_addr == 0 {
                return false;
            }
            let dst = resolve(resolver, v.dst_addr, v.time);
            dst != NO_AGENT
                && log.agents.slot(dst).is_some_and(|a| i64::from(a.id) == species)
        }
        "has_spawned_minion" => {
            // HasSpawnedMinion(species, master, time, eps):
            // 该 master 有该物种 agent 且 |FirstAware - time| < eps。
            let species = c.args[0].as_i64().unwrap_or(0) as i32;
            let side = c
                .args
                .get(1)
                .and_then(|a| a.get("agent"))
                .and_then(|x| x.as_str())
                .unwrap_or("dst");
            let eps = c.args.get(3).and_then(|x| x.as_i64()).unwrap_or(SERVER_DELAY);
            let t = v.time
                + c.args
                    .get(2)
                    .and_then(|a| a.get("t"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
            let addr = side_addr(v, side);
            if addr == 0 {
                return false;
            }
            let master = resolve(resolver, addr, v.time);
            let rows = idx.spawn_of_species.get(&species);
            rows.is_some_and(|rows| {
                rows.iter().any(|&ev| {
                    let ev2 = view_of(log, ev);
                    if ev2.time - t >= eps {
                        return false;
                    }
                    let src = resolve(resolver, ev2.src_addr, ev2.time);
                    same_identity(log, src, master)
                })
            })
        }
        "two_skill_damage" => {
            let mode = c.args[0].as_str().unwrap_or("a_not_b");
            let a = c.args[1].as_i64().unwrap_or(0);
            let b = c.args[2].as_i64().unwrap_or(0);
            let win = c.args[3].as_i64().unwrap_or(2000);
            let mut has = |skill: i64| -> bool {
                rows_of(&idx.damage_by_id, skill).iter().any(|&ev| {
                    let ev2 = view_of(log, ev);
                    if ev2.src_addr == 0 {
                        return false;
                    }
                    let from = resolve(resolver, ev2.src_addr, ev2.time);
                    same_identity(log, from, key_agent) && (ev2.time - v.time).abs() < win
                })
            };
            let (ha, hb) = (has(a), has(b));
            match mode {
                "a_not_b" => ha && !hb,
                "either_both" => (!ha && !hb) || (ha && hb),
                _ => false,
            }
        }
        "recent_remove_all" => {
            // 只算 found;neg 由函数尾统一应用(此前双重取反导致行为反转:
            // !FindRelatedEvents(...).Any() 判据与 C# 相反 —— 修后
            // 29560 SpitefulSpirit 对拍一致)。
            let buff = c.args[0].as_i64().unwrap_or(0);
            let t_off = c.args[1].as_i64().unwrap_or(0);
            let eps = c.args.get(2).and_then(|x| x.as_i64()).unwrap_or(SERVER_DELAY);
            let t = v.time + t_off;
            let rows = rows_of(&idx.remove_all_by_buff, buff);
            rows.iter().any(|&ev| (view_of(log, ev).time - t).abs() < eps)
        }
        "mine_detonation" => {
            let if_found = c.args[0].as_bool().unwrap_or(false);
            let guids: Vec<&str> = c.args[1]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|g| g.as_str())
                .collect();
            let mut matched = false;
            'outer: for g in &guids {
                let eid = log
                    .metadata
                    .effect_guid_by_effect_id
                    .iter()
                    .find(|(_, info)| &info.guid_hex == g)
                    .map(|(id, _)| *id);
                let Some(eid) = eid else { continue };
                for &ev in rows_of(&idx.effect_by_id, eid) {
                    let ev2 = view_of(log, ev);
                    if ev2.src_addr == v.src_addr
                        && ev2.src_addr != 0
                        && ev2.end_time.is_some_and(|e| (e - v.time).abs() < SERVER_DELAY)
                    {
                        matched = true;
                        break 'outer;
                    }
                }
            }
            matched == if_found
        }
        "buff_apply_window" => {
            // Advance/StandYourGround:FindRelatedEvents(
            // GetBuffApplyDataByIDBySrc(Buff, dst), time) ±10ms;
            // apply.To.Is(dst) 且(时长窗或计数)。
            let buff = c.args[0].as_i64().unwrap_or(0);
            let min_dur = c.args[2].as_i64().unwrap_or(0);
            let max_dur = c.args[3].as_i64().unwrap_or(0);
            let min_count = c.args[4].as_i64().unwrap_or(1);
            let t = v.time;
            if std::env::var("EI_DEBUG_INSTANT").is_ok() && buff == 1122 {
                eprintln!(
                    "    baw t={} key={:?} rows={}",
                    t,
                    key_agent,
                    rows_of(&idx.apply_by_buff, buff).len()
                );
                for &ev in rows_of(&idx.apply_by_buff, buff) {
                    let ev2 = view_of(log, ev);
                    if (ev2.time - t).abs() < 50 {
                        let by = if ev2.by_addr != 0 { resolve(resolver, ev2.by_addr, ev2.time) } else { NO_AGENT };
                        eprintln!(
                            "      apply t={} by={:?} to={:x} dur={}",
                            ev2.time, by, ev2.to_addr, ev2.applied_duration
                        );
                    }
                }
            }
            let rows = rows_of(&idx.apply_by_buff, buff);
            let mut count = 0i64;
            for &ev in rows {
                let ev2 = view_of(log, ev);
                if (ev2.time - t).abs() >= SERVER_DELAY {
                    continue;
                }
                if ev2.by_addr == 0 {
                    continue;
                }
                let by = resolve(resolver, ev2.by_addr, ev2.time);
                if !same_identity(log, by, key_agent) {
                    continue;
                }
                let CombatEvent::BuffApply(f) = &log.events[ev] else {
                    continue;
                };
                let to = resolve(resolver, f.apply.base.to, ev2.time);
                if !same_identity(log, to, key_agent) {
                    continue;
                }
                let d = i64::from(f.applied_duration);
                let in_dur = max_dur == 0
                    || (d + SERVER_DELAY >= min_dur && d - SERVER_DELAY <= max_dur);
                if in_dur {
                    count += 1;
                }
            }
            count >= min_count
        }
        "sadistic_damage_window" => {
            let skill = c.args[0].as_i64().unwrap_or(0);
            let window = 10_000 - v.removed_duration;
            let t = v.time;
            let rows = rows_of(&idx.damage_by_id, skill);
            rows.iter().any(|&ev| {
                let ev2 = view_of(log, ev);
                if ev2.src_addr == 0 {
                    return false;
                }
                let from = resolve(resolver, ev2.src_addr, ev2.time);
                same_identity(log, from, key_agent) && ev2.time >= t - window && ev2.time <= t
            })
        }
        "no_animated_cast" => {
            // !IsCasting(skill, key agent, time + off, eps):该技能动画施法
            // 窗口覆盖 key agent(C# CombatDataHelpers.cs:22-26 +
            // AnimatedCastEvent.IntersectsActualCastWindow:171-174)。
            let skill = c.args[0].as_i64().unwrap_or(0);
            let t = v.time + c.args[1].as_i64().unwrap_or(0);
            let eps = c.args[2].as_i64().unwrap_or(SERVER_DELAY);
            let rows = rows_of(&idx.animated_by_id, skill);
            let casting = rows.iter().any(|&ev| {
                let ev2 = view_of(log, ev);
                if ev2.src_addr == 0 {
                    return false;
                }
                let from = resolve(resolver, ev2.src_addr, ev2.time);
                same_identity(log, from, key_agent)
                    && t >= ev2.time - eps
                    && ev2.time + ev2.duration + eps >= t
            });
            !casting
        }
        "is_casting" => {
            let skill = c.args[0].as_i64().unwrap_or(0);
            let side = c
                .args
                .get(1)
                .and_then(|a| a.get("agent"))
                .and_then(|x| x.as_str())
                .unwrap_or("to");
            let t = v.time + c.args[2].as_i64().unwrap_or(0);
            let addr = side_addr(v, side);
            if addr == 0 {
                return false;
            }
            let agent = resolve(resolver, addr, v.time);
            let rows = rows_of(&idx.animated_by_id, skill);
            rows.iter().any(|&ev| {
                let ev2 = view_of(log, ev);
                if ev2.src_addr == 0 {
                    return false;
                }
                let from = resolve(resolver, ev2.src_addr, ev2.time);
                same_identity(log, from, agent)
                    && t >= ev2.time - SERVER_DELAY
                    && ev2.time + ev2.duration + SERVER_DELAY >= t
            })
        }
        "secondary_effect_same_src" => {
            // TryGetEffectEventsByGUID(guid).Any(other != evt &&
            // same key-agent identity && |other.Time - off - evt.Time| < eps)
            let guid = c.args[0].as_str().unwrap_or("");
            let off = c.args[1].as_i64().unwrap_or(0);
            let eps = c.args[2].as_i64().unwrap_or(SERVER_DELAY);
            let eid = log
                .metadata
                .effect_guid_by_effect_id
                .iter()
                .find(|(_, info)| info.guid_hex == guid)
                .map(|(id, _)| *id);
            let Some(eid) = eid else { return false };
            let rows = rows_of(&idx.effect_by_id, eid);
            rows.iter().any(|&ev| {
                let ev2 = view_of(log, ev);
                if ev2.src_addr == 0 {
                    return false;
                }
                let from = resolve(resolver, ev2.src_addr, ev2.time);
                same_identity(log, from, key_agent)
                    && (ev2.time - off - v.time).abs() < eps
            })
        }
        "secondary_effect_inverted_src" => {
            // other 用 GetOtherAgent(dst 侧)与 key agent(src)比较。
            let guid = c.args[0].as_str().unwrap_or("");
            let off = c.args[1].as_i64().unwrap_or(0);
            let eps = c.args[2].as_i64().unwrap_or(SERVER_DELAY);
            let eid = log
                .metadata
                .effect_guid_by_effect_id
                .iter()
                .find(|(_, info)| info.guid_hex == guid)
                .map(|(id, _)| *id);
            let Some(eid) = eid else { return false };
            let rows = rows_of(&idx.effect_by_id, eid);
            rows.iter().any(|&ev| {
                let ev2 = view_of(log, ev);
                if ev2.dst_addr == 0 {
                    return false;
                }
                let other = resolve(resolver, ev2.dst_addr, ev2.time);
                same_identity(log, other, key_agent)
                    && (ev2.time - off - v.time).abs() < eps
            })
        }
        "no_secondary_same_src_same_position" => {
            let guid = c.args[0].as_str().unwrap_or("");
            let off = c.args[1].as_i64().unwrap_or(0);
            let eps = c.args[2].as_i64().unwrap_or(SERVER_DELAY);
            let eid = log
                .metadata
                .effect_guid_by_effect_id
                .iter()
                .find(|(_, info)| info.guid_hex == guid)
                .map(|(id, _)| *id);
            let Some(eid) = eid else { return true };
            let rows = rows_of(&idx.effect_by_id, eid);
            let found = rows.iter().any(|&ev| {
                let ev2 = view_of(log, ev);
                if ev2.src_addr == 0 {
                    return false;
                }
                let from = resolve(resolver, ev2.src_addr, ev2.time);
                let dpos = (ev2.position.0 - v.position.0).powi(2)
                    + (ev2.position.1 - v.position.1).powi(2)
                    + (ev2.position.2 - v.position.2).powi(2);
                same_identity(log, from, key_agent)
                    && (ev2.time - off - v.time).abs() < eps
                    && dpos < 1e-6
            });
            !found
        }
        "relic_claw_no_recent_single" => {
            let buff = c.args[0].as_i64().unwrap_or(0);
            let rows = rows_of(&idx.remove_single_by_buff, buff);
            let has = rows.iter().any(|&ev| {
                let ev2 = view_of(log, ev);
                if ev2.to_addr == 0 {
                    return false;
                }
                let to = resolve(resolver, ev2.to_addr, ev2.time);
                same_identity(log, to, key_agent)
                    && ev2.time >= v.time
                    && ev2.time - v.time < SERVER_DELAY
                    && ev2.buff_instance != v.buff_instance
            });
            !has
        }
        _ => true,
    };
    if c.neg {
        !res
    } else {
        res
    }
}
// ===== 时间吸附(GetTime, InstantCastFinder.cs:117-129)=====

fn adjusted_time(
    log: &ParsedLog,
    f: &Finder,
    evt_time: i64,
    key_agent: AgentId,
    idx: &TriggerIndex<'_>,
) -> i64 {
    let time = evt_time + f.time_offset;
    if (f.before_swap || f.after_swap)
        && let Some(swaps) = idx.swap_by_caster.get(&key_agent)
    {
        for &ev in swaps {
            let swap = &log.events[ev];
            if let CombatEvent::WeaponSwap(s) = swap
                && (s.time - time).abs() < SERVER_DELAY / 2
            {
                return if f.before_swap {
                    std::cmp::min(s.time - 1, time)
                } else {
                    std::cmp::max(s.time + 1, time)
                };
            }
        }
    }
    time
}

// ===== 主入口 =====

/// 加载 finder 表文件(不入 Content —— CLI 内容目录直接读)。
pub fn load_finder_doc(content_dir: &std::path::Path) -> Result<Value, crate::BuildError> {
    let p = content_dir.join("instant-cast-finders.json");
    serde_json::from_str(
        &std::fs::read_to_string(&p)
            .map_err(|e| crate::BuildError::Content(format!("read {p:?}: {e}")))?,
    )
    .map_err(|e| crate::BuildError::Content(format!("parse {p:?}: {e}")))
}

/// 计算合成事件 + skillData 标记。
///
/// `finders`:instant-cast-finders.json 内容(generic/helpers 两组);
/// `players`:参与注册的 agent(C# = GetAgentByType(Player) +
/// NonSquadPlayer,CombatData.cs:450-451)。
#[allow(clippy::too_many_lines)]
pub fn compute_instants(
    log: &ParsedLog,
    content: &Content,
    gw2_build: u64,
    finders: &Value,
) -> InstantOut {
    let mut out = InstantOut::default();
    let idx = build_index(log);

    // ---- 注册(ProfHelper.GetProfessionInstantCastFinders)----
    // players = Player + NonSquadPlayer 根 agent(C# CombatData.cs:450-451)
    let mut players: Vec<AgentId> = Vec::new();
    for ty in [gw2ei_model::AgentType::Player, gw2ei_model::AgentType::NonSquadPlayer] {
        let mut table = log.agents.clone();
        players.extend(table.agents_by_type(ty));
    }
    let mut base_specs: BTreeSet<String> = BTreeSet::new();
    let mut specs: BTreeSet<String> = BTreeSet::new();
    for p in &players {
        if let Some(a) = log.agents.slot(*p)
            && a.agent_type.is_player()
        {
            specs.insert(a.spec.csharp_name().to_string());
            base_specs.insert(a.base_spec.csharp_name().to_string());
        }
    }
    // 注册序 = C# ProfHelper.GetProfessionInstantCastFinders
    // (ProfHelper.cs:342-513):_genericNeedsToBeBeforeTheRest… +
    // _genericInstantCastFinders 两通用组在前,base specs(Distinct 首见序)
    // 居中,spec 表最后。同刻同 caster 的 instant 输出序 = finder 注册序
    // (.NET HashSet 枚举序 = 插入序,实测 3 进程一致)——
    // 通用表必须在职业表前(9284 Flame Blast sigil 等跨表同刻决胜)。
    let mut registered: Vec<(String, Finder)> = Vec::new();
    let push_table = |group: &str, spec: &str, doc: &Value, reg: &mut Vec<(String, Finder)>| {
        let entries = doc["helpers"]
            .get(group)
            .and_then(|g| g.get(spec))
            .and_then(|v| v.as_array());
        let Some(entries) = entries else { return };
        for (i, e) in entries.iter().enumerate() {
            let label = format!("{group}/{spec}[{i}]");
            if let Some(f) = decode_finder(e, &label) {
                reg.push((label, f));
            }
        }
    };
    // 通用表(无条件;ProfHelper.cs:345-346 两组合并)
    if let Some(generic) = finders["generic"].as_array() {
        for (i, e) in generic.iter().enumerate() {
            let label = format!("generic[{i}]");
            if let Some(f) = decode_finder(e, &label) {
                registered.push((label, f));
            }
        }
    }
    for spec in &base_specs {
        push_table("base", spec, finders, &mut registered);
    }
    for spec in &specs {
        push_table("spec", spec, finders, &mut registered);
    }

    let evtc_build = i64::from(log.arc_version.0);
    for (label, f) in &registered {
        let watch = std::env::var("EI_DEBUG_INSTANT").is_ok()
            && matches!(f.skill_id, -29 | -28 | 9153 | 10238 | 62671 | 12689 | 12667 | 42163 | 79359 | 65418 | 56873 | 5780 | 26261 | 71252 | 29560 | 5812 | 5933);
        if watch {
            eprintln!(
                "finder {label}: skill={} kind={} avail={} icd={} bmin={} bmax={} emin={} emax={} na={}",
                f.skill_id,
                f.kind,
                available(f, gw2_build, evtc_build, &idx),
                f.icd,
                f.build_min,
                f.build_max,
                f.evtc_min,
                f.evtc_max,
                f.not_accurate
            );
        }
        if !available(f, gw2_build, evtc_build, &idx) {
            continue;
        }
        // 事件面缺失的 finder(extension/marker 等;数据缺失 = C# enable
        // 条件不满足 → 不 Available → 不打标不触发)在打标前跳过 —— 与
        // C# 一致:EXTHealing/EXTBarrier(扩展流)、Marker、BuffExtend、
        // MinionCastCast 在样本无对应数据时不产生 proc 标记(30784
        // Invigorating Bond 等 EXT 行据此不标 Trait)。
        if matches!(
            f.kind.as_str(),
            "EXTHealingCastFinder"
                | "EXTBarrierCastFinder"
                | "MarkerCastFinder"
                | "BuffExtendCastFinder"
                | "MinionCastCastFinder"
        ) {
            out.skipped.push(format!("{label} {}", f.kind));
            continue;
        }
        // 打标(ComputeInstantCastEventsFromFinders:225-243)
        if f.not_accurate {
            out.not_accurate.insert(f.skill_id);
        }
        match f.origin {
            Origin::Trait => {
                out.trait_proc.insert(f.skill_id);
            }
            Origin::Gear => {
                out.gear_proc.insert(f.skill_id);
            }
            Origin::Unconditional => {
                out.unconditional_proc.insert(f.skill_id);
            }
            Origin::Skill => {}
        }
        let Some((ty, rows)) = trigger_events(&idx, f) else {
            // EngineerKit 无统一触发面:per-caster 双游标处理在下方 kind
            // 分派(trigger_events 对 kind 返回 None 属正常)。
            if f.kind == "EngineerKit" {
                run_engineer_kit(log, content, &idx, f, &mut out);
            }
            continue;
        };
        if std::env::var("EI_DEBUG_INSTANT").is_ok()
            && matches!(f.skill_id, -29 | 9153 | 10238 | 62671)
        {
            eprintln!("  trig {}({}) rows={} ty={ty}", f.skill_id, f.kind, rows.len());
            if let Some(g) = arg1_guid(f) {
                let eid = log
                    .metadata
                    .effect_guid_by_effect_id
                    .iter()
                    .find(|(_, info)| info.guid_hex == g)
                    .map(|(id, _)| *id);
                eprintln!("    guid={g} eid={eid:?}");
            }
            for &ev in rows.iter().take(5) {
                let v = view_of(log, ev);
                eprintln!(
                    "      t={} src={:x} dst={:x} around={} dur={} end={:?}",
                    v.time, v.src_addr, v.dst_addr, v.around_dst, v.duration, v.end_time
                );
            }
        }
        // MinionSpawn:事件行由 species 表出(触发事件跨 master 分组)。
        if f.kind == "MinionSpawnCastFinder" {
            run_minion_spawn(log, &idx, f, &mut out);
            continue;
        }
        run_generic(log, &idx, f, ty, rows, &mut out);
    }
    if std::env::var("EI_DEBUG_INSTANT").is_ok() {
        let effect_in_events = log
            .events
            .iter()
            .filter(|e| matches!(e, CombatEvent::Effect(_)))
            .count();
        let guid_evts = log
            .events
            .iter()
            .filter(|e| matches!(e, CombatEvent::GuidEffect(_)))
            .count();
        eprintln!(
            "dbg: effect_in_events={} guid_in_events={} effect_count={} guid_table={} players={} base={:?} spec={:?} registered={}",
            effect_in_events,
            guid_evts,
            idx.effect_count,
            log.metadata.effect_guid_by_effect_id.len(),
            players.len(),
            base_specs,
            specs,
            registered.len()
        );
        let mut by_skill: BTreeMap<i64, usize> = BTreeMap::new();
        for ls in out.lines.values() {
            for l in ls {
                *by_skill.entry(l.skill_id).or_insert(0) += 1;
            }
        }
        eprintln!("instant lines: {} skills {} | skipped: {}", out.lines.values().map(|v| v.len()).sum::<usize>(), by_skill.len(), out.skipped.len());
        for (k, v) in by_skill.iter().take(60) {
            eprintln!("  {k} x {v}");
        }
    }
    out
}

/// 通用骨架:按 key agent 分组 → 组内时间序 ICD → emit。
#[allow(clippy::too_many_lines)]
fn run_generic(
    log: &ParsedLog,
    idx: &TriggerIndex<'_>,
    f: &Finder,
    ty: &str,
    rows: &[usize],
    out: &mut InstantOut,
) {
    // 组:caster(已含 minions 展开)→ 行列表(时间序)。
    let mut groups: BTreeMap<AgentId, Vec<usize>> = BTreeMap::new();
    let mut resolver = log.agents.clone();
    let watch = std::env::var("EI_DEBUG_INSTANT").is_ok()
        && matches!(f.skill_id, -29 | 9153 | 10238 | 62671 | 12689 | -28);
    let mut n_unresolved = 0usize;
    for &ev in rows {
        let v = view_of(log, ev);
        let key_addr = key_addr_of(f, &v);
        if key_addr == 0 {
            n_unresolved += 1;
            continue;
        }
        let key_agent = resolve(&mut resolver, key_addr, v.time);
        if key_agent == NO_AGENT {
            n_unresolved += 1;
            continue;
        }
        let caster = group_key(log, f, key_agent);
        if caster == NO_AGENT {
            continue;
        }
        groups.entry(caster).or_default().push(ev);
    }
    if watch {
        eprintln!(
            "  grp {}({}) rows={} groups={} unresolved={}",
            f.skill_id,
            f.kind,
            rows.len(),
            groups.len(),
            n_unresolved
        );
    }
    // MinionCommand 需 checker:To 是 species 且带 master —— key agent 是
    // minion 的 apply 的 to(C# MinionCommandCastFinder checker 在
    // BuffApplyEvent 上:evt.To.Type != Volatile && IsSpecies && Master !=
    // null)。检查在组内条件处理中按 kind 特判。
    let _ = ty;
    for (caster, evs) in &groups {
        let mut last_time = i64::MIN;
        let mut resolver2 = log.agents.clone();
        let mut n_emit = 0usize;
        for &ev in evs {
            let v = view_of(log, ev);
            let key_addr = key_addr_of(f, &v);
            let key_agent = resolve(&mut resolver2, key_addr, v.time);
            let agent_name = if watch {
                let k = if key_agent == NO_AGENT { None } else { log.agents.slot(key_agent) };
                format!(
                    "{:?}/{}",
                    key_agent,
                    k.map(|a| a.spec.csharp_name().to_string()).unwrap_or_default()
                )
            } else {
                String::new()
            };
            // kind 级默认 checker(MinionCommand 的 species/master 校验)
            if f.kind == "MinionCommandCastFinder" {
                let species = arg1_int(f);
                let ok = key_agent != NO_AGENT
                    && log
                        .agents
                        .slot(key_agent)
                        .is_some_and(|a| i64::from(a.id) == species && a.master != NO_AGENT);
                if !ok {
                    continue;
                }
            }
            if !check_conds(log, idx, f, &v, key_agent, *caster, &mut resolver2) {
                if watch {
                    eprintln!("    cond-fail {} t={} key={}", f.skill_id, v.time, agent_name);
                }
                continue;
            }
            let t = v.time;
            if t.saturating_sub(last_time) < f.icd {
                last_time = t;
                continue;
            }
            last_time = t;
            n_emit += 1;
            let emit_time = adjusted_time(log, f, t, key_agent, idx);
            out.lines
                .entry(*caster)
                .or_default()
                .push(InstantLine { time: emit_time, skill_id: f.skill_id });
        }
        let _ = n_emit;
        if watch {
            eprintln!(
                "    caster {:?} events={} emitted={}",
                caster,
                evs.len(),
                n_emit
            );
        }
    }
}

/// MinionSpawn(MinionSpawnCastFinder.cs:19-51):species agents(带 master)
/// 的 Spawn 事件;按 final master 分组 + ICD;emit 时间 = spawn.Time(不经
/// GetTime)。
fn run_minion_spawn(
    log: &ParsedLog,
    idx: &TriggerIndex<'_>,
    f: &Finder,
    out: &mut InstantOut,
) {
    let species_ids: Vec<i32> = match &f.arg1 {
        Some(MArg::Int(v)) => vec![*v as i32],
        Some(MArg::Ids(v)) => v.iter().map(|x| *x as i32).collect(),
        _ => return,
    };
    let mut spawn_rows = Vec::new();
    for id in &species_ids {
        if let Some(r) = idx.spawn_of_species.get(id) {
            spawn_rows.extend(r.iter().copied());
        }
    }
    // species agent id(agent.id == species)且 master != null
    let mut by_master: BTreeMap<AgentId, Vec<i64>> = BTreeMap::new();
    let mut resolver = log.agents.clone();
    for &ev in &spawn_rows {
        // spawn rows 已按时间序
        let v = view_of(log, ev);
        let a = resolve(&mut resolver, v.src_addr, v.time);
        if a == NO_AGENT {
            continue;
        }
        let m = log.agents.final_master(a);
        if m == a {
            continue;
        }
        by_master.entry(m).or_default().push(v.time);
    }
    for (master, mut times) in by_master {
        // spawn 行按 species 分桶跨行合并可能乱序 —— ICD 前按时间排
        times.sort_unstable();
        times.dedup();
        let mut last_time = i64::MIN;
        for t in times {
            if t.saturating_sub(last_time) < f.icd {
                last_time = t;
                continue;
            }
            last_time = t;
            out.lines
                .entry(master)
                .or_default()
                .push(InstantLine { time: t, skill_id: f.skill_id });
        }
    }
}

/// EngineerKitFinder(EngineerHelper.cs:16-84)手工移植:per-caster 双游标
/// (swap/动画 cast 索引),检查器在 WeaponSwapCastFinder 的
/// SwappedTo==KitSet(2) 之上叠加:
///
/// ```text
/// 无 API bundle_skills → false
/// nextSwap = 第一个 Time > swap.Time+10 的 swap(游标起搜)
/// 扫描动画 cast(游标起):cast.Time >= swap.Time+75 时
///   cast.Time >= nextSwap.Time → (游标回退 i-1)false
///   否则 bundle 含 cast.Skill → (游标= i+1)true
/// ```
///
/// C# 检查器链 .All 短路:非 kit-set swap 不推进游标。发射时间经
/// GetTime(BeforeWeaponSwap)= min(swap.Time-1, time)。
#[allow(clippy::too_many_lines)]
fn run_engineer_kit(
    log: &ParsedLog,
    content: &Content,
    idx: &TriggerIndex<'_>,
    f: &Finder,
    out: &mut InstantOut,
) {
    // C# EngineerKitFinder 对 Player 型 agent 处理(WeaponSwapCastFinder
    // ComputeInstantCast 用 GetAgentByType(Player) —— 与引擎玩家注册面
    // 一致;swap 行按 resolved caster 分组)。
    let kit = f.skill_id;
    let api = content.skills_api.get(&kit);
    let Some(bundles) = api.and_then(|a| a.bundle_skills.as_ref()) else {
        // C#:skill.ApiSkill == null || BundleSkills == null → 恒 false
        return;
    };
    let mut swap_state: BTreeMap<AgentId, usize> = BTreeMap::new();
    let mut cast_state: BTreeMap<AgentId, usize> = BTreeMap::new();
    let mut last_time: BTreeMap<AgentId, i64> = BTreeMap::new();
    // 全 caster 覆盖:C# 遍历 GetAgentByType(Player) 的 swap 数据;这里按
    // 索引里带 swap 行的 caster(等同 player + NonSquadPlayer)。
    let casters: Vec<AgentId> = idx.swap_by_caster.keys().copied().collect();
    for caster in casters {
        let Some(swaps) = idx.swap_by_caster.get(&caster) else { continue };
        let casts = idx.animated_by_caster.get(&caster).cloned().unwrap_or_default();
        let swap_i = swap_state.entry(caster).or_default();
        let cast_i = cast_state.entry(caster).or_default();
        let lt = last_time.entry(caster).or_insert(i64::MIN);
        for &sw in swaps {
            let CombatEvent::WeaponSwap(s) = &log.events[sw] else { continue };
            if i64::from(s.swapped_to) != 2 {
                // 基类 checker(SwappedTo == KitSet)短路 → 检查器未跑,
                // 游标不动(C# .All 短路)
                continue;
            }
            // nextSwap 搜索(游标起;swap 行按时间升序)
            let mut next_swap_time = i64::MAX;
            let mut j = *swap_i;
            while j < swaps.len() {
                if let CombatEvent::WeaponSwap(ns) = &log.events[swaps[j]]
                    && ns.time > s.time + SERVER_DELAY
                {
                    *swap_i = j;
                    next_swap_time = ns.time;
                    break;
                }
                j += 1;
            }
            // 动画 cast 扫描
            let mut ok = false;
            let mut at_least_one = false;
            let mut i = *cast_i;
            while i < casts.len() {
                let ev = casts[i];
                let (ct, cskill) = match &log.events[ev] {
                    CombatEvent::AnimatedCast(c) | CombatEvent::GadgetInteract(c) => {
                        (c.time, i64::from(c.skill_id))
                    }
                    CombatEvent::Emote(c) => (c.base.time, i64::from(c.base.skill_id)),
                    CombatEvent::BundlePickUp(c) => (c.base.time, i64::from(c.base.skill_id)),
                    _ => break,
                };
                if ct >= s.time + WEAPON_SWAP_DELAY {
                    if ct >= next_swap_time {
                        if at_least_one {
                            *cast_i = i.saturating_sub(1);
                        }
                        break;
                    }
                    at_least_one = true;
                    if bundles.contains(&cskill) {
                        *cast_i = i + 1;
                        ok = true;
                        break;
                    }
                }
                i += 1;
            }
            if !ok {
                continue;
            }
            // ICD(组内 swap 时间)
            if s.time.saturating_sub(*lt) < f.icd {
                *lt = s.time;
                continue;
            }
            *lt = s.time;
            // GetTime(BeforeWeaponSwap):±5ms 内换武器吸附 → min(sw-1, t)
            let emit = std::cmp::min(s.time - 1, s.time + f.time_offset);
            out.lines
                .entry(caster)
                .or_default()
                .push(InstantLine { time: emit, skill_id: kit });
        }
    }
}

