//! 真实样例集成测试（skip-if-missing）。
//!
//! 样例 `rust/testdata/20260530-205048.zevtc` 是 SBR 白名单样例的拷贝
//!（详见 `rust/testdata/README.md`），gitignore 忽略不入库；读不到时
//! eprintln + return（skip 模式），CI 无样例也能过。
//!
//! 断言分层：
//! 1. **raw 三区计数**（agent=410 / skill=740 / combat=320684）—— 与
//!    SBR `evtc-format` 的 golden 一致，交叉验证二进制布局；
//! 2. **事件化冒烟**：`build_combat_data` 不报 `UnsupportedArcBuild`，
//!    各桶计数 eprintln 输出（数值本阶段不硬断言，P2 黄金对拍时锁定）。
//!
//! 60s 硬超时由外层验证命令控制（单遍流式解析，避免多余拷贝）。

use gw2ei_model::{build_combat_data, CategoryCounts, CombatEvent};
use gw2ei_parse::read_evtc_log;

fn sample_path() -> std::path::PathBuf {
    // cargo test 的 cwd 是 crate 目录（crates/gw2ei-model）。
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("testdata")
        .join("20260530-205048.zevtc")
}

fn load_sample() -> Option<gw2ei_parse::EvtcRawLog> {
    let path = sample_path();
    let Ok(log) = read_evtc_log(&path) else {
        eprintln!(
            "skipping golden sample test: {} not found (not committed; copy from SBR testdata)",
            path.display()
        );
        return None;
    };
    Some(log)
}

#[test]
fn golden_sample_raw_counts_match_sbr() {
    let Some(log) = load_sample() else { return };
    // 与 SBR evtc-format golden 交叉验证的 raw 计数
    assert_eq!(log.stats.raw_agent_count, 410);
    assert_eq!(log.stats.raw_skill_count, 740);
    assert_eq!(log.stats.raw_combat_count, 320_684);
    // 守恒：kept + 丢弃 + ArcBuild 消费行 = raw 区条数
    assert_eq!(
        log.stats.kept_combat_count as u64
            + log.stats.discarded.total()
            + log.stats.arc_build_consumed,
        320_684
    );
    // header 快照信息
    eprintln!(
        "header_build={} revision={} id={} map_id={} kept={} discarded={} log_end={}ms arc={:?} gw2={}",
        log.header_build,
        log.revision,
        log.id,
        log.map_id,
        log.stats.kept_combat_count,
        log.stats.discarded.total(),
        log.log_end_time,
        log.arc_version,
        log.gw2_build,
    );
}

#[test]
fn golden_sample_eventizes_without_errors() {
    let Some(log) = load_sample() else { return };
    let data = build_combat_data(log).expect("new-generation sample must eventize");
    eprintln!("===== per-category event counts =====");
    dump_counts(&data.counts);
    eprintln!("===== unsupported (P1 untyped) =====");
    for (kind, n) in &data.unsupported {
        eprintln!("{:<20} {n}", kind.label());
    }
    eprintln!(
        "===== totals: events={} agents={} ids={} =====",
        data.events.len(),
        data.per_agent_counts.len(),
        data.per_id_counts.len()
    );
    // 排序后的第一个与最后一个事件时间应单调（events 时间序）
    let first = data.events.first().and_then(CombatEvent::time);
    let last = data.events.last().and_then(CombatEvent::time);
    if let (Some(f), Some(l)) = (first, last) {
        eprintln!("time span: [{f}, {l}]");
        assert!(f <= l);
    }
}

fn dump_counts(c: &CategoryCounts) {
    let pairs = [
        ("direct_health_damage", c.direct_health_damage),
        ("non_direct_health_damage", c.non_direct_health_damage),
        ("no_damage_health_damage", c.no_damage_health_damage),
        ("breakbar_damage", c.breakbar_damage),
        ("breakbar_recovery", c.breakbar_recovery),
        ("crowd_control", c.crowd_control),
        ("stun_break", c.stun_break),
        ("buff_apply", c.buff_apply),
        ("buff_extension", c.buff_extension),
        ("buff_stack_active", c.buff_stack_active),
        ("buff_stack_deactive", c.buff_stack_deactive),
        ("buff_remove_single", c.buff_remove_single),
        ("buff_remove_all", c.buff_remove_all),
        ("buff_remove_manual", c.buff_remove_manual),
        ("animated_cast", c.animated_cast),
        ("emote", c.emote),
        ("gadget_interact", c.gadget_interact),
        ("bundle_pick_up", c.bundle_pick_up),
        ("weapon_swap", c.weapon_swap),
        ("enter_combat", c.enter_combat),
        ("exit_combat", c.exit_combat),
        ("alive", c.alive),
        ("dead", c.dead),
        ("down", c.down),
        ("spawn", c.spawn),
        ("despawn", c.despawn),
        ("health_update", c.health_update),
        ("barrier_update", c.barrier_update),
        ("max_health_update", c.max_health_update),
        ("team_change", c.team_change),
        ("targetable", c.targetable),
        ("visibility", c.visibility),
        ("breakbar_state", c.breakbar_state),
        ("breakbar_percent", c.breakbar_percent),
        ("glider", c.glider),
        ("last90_before_down", c.last90_before_down),
        ("movement", c.movement),
        ("metadata_singletons", c.metadata_singletons),
        ("metadata_list", c.metadata_list),
        ("metadata_guid", c.metadata_guid),
        ("dropped_by_cs", c.dropped_by_cs),
    ];
    for (name, n) in pairs {
        eprintln!("{name:<24} {n}");
    }
}

/// P3b 事件面锚点:effect/missile 事件化的保留计数(与 instant 引擎
/// 消费同源;Split 世代 60-63)。数值来自 20260530-205048.zevtc 实测
/// (2026-09-07 P3b 对拍锁定)。
#[test]
fn golden_sample_effect_missile_event_counts() {
    let Some(log) = load_sample() else { return };
    let data = build_combat_data(log).expect("eventize");
    let mut effect = 0usize;
    let mut missile = 0usize;
    let mut guid_effect = 0usize;
    for e in &data.events {
        match e {
            CombatEvent::Effect(_) => effect += 1,
            CombatEvent::Missile(_) => missile += 1,
            CombatEvent::GuidEffect(_) => guid_effect += 1,
            _ => {}
        }
    }
    eprintln!("effect={effect} missile={missile} guid_effect={guid_effect}");
    // 保留数锁定(Split 世代;OnNonStaticPlatform 释放行已滤)
    assert_eq!(effect, 30_477);
    assert_eq!(missile, 2314);
    assert_eq!(guid_effect, 0);
}
