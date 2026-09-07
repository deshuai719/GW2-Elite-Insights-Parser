//! 统计归约与公式层 —— EI `EIData/Statistics/{BuffStatistics,BuffByActorStatistics}
//! .cs` + `BuffDistribution.cs` + `BuffGraph.cs` 的数值面（对 agent 泛型）。
//!
//! 公式锚点（C# 行号见各函数注释）：
//! - 段归约（BuffSimulationItem*.SetBuffDistributionItem + BuffDistributionItem）：
//!   主时长 → Value；`hasSeed`（seed≠src 身份）→ Extended 记 seed + src
//!   unknown 时 UnknownExtension 记 seed；`isExtension` → Extension 记 src。
//!   每源 6 向累计（value/overstack/waste/unknownExt/extension/extended）。
//! - waste/overstack 事件按 `start<=time<=end`（闭区间）计入 phase。
//! - presence（GetBuffPresence）= 任意栈在场窗（段窗 clamp）；per-src presence
//!   = 该 src 有 ≥1 栈在场窗（Duration 型仅头栈）。
//! - 百分比公式 + `Math.Round(x,3)` 银行家舍入 → `round3`。
//!
//! JSON 数值容差：求和顺序差异导致的 f64 尾差由对拍层按 1e-9 相对容差处理
//! （见 golden_compare；`shorten_numbers` 前已 round3 —— 绝大多数逐位相等）。

use std::collections::BTreeMap;

use crate::sim::{AgentKey, SimResult};

/// 每源 6 向累计（BuffDistributionItem.cs:6-13）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Acc {
    pub value: i64,
    pub overstack: i64,
    pub waste: i64,
    pub unknown_extension: i64,
    pub extension: i64,
    pub extended: i64,
}

/// `BuffDistribution.GetUptime`（Σ Value 不分源）。
pub fn total_value<A: Ord>(acc: &BTreeMap<A, Acc>) -> i64 {
    acc.values().map(|a| a.value).sum()
}

/// phase 区间 clamp 的公共实现（BuffSimulationItem.GetClampedDuration：
/// `start>=end || start>=End || Start>=end → 0`；闭区间相交）。
pub fn clamped(start: i64, end: i64, item_start: i64, item_end: i64) -> i64 {
    if start >= end || start >= item_end || item_start >= end {
        return 0;
    }
    let lo = item_start.max(start);
    let hi = item_end.min(end);
    (hi - lo).max(0)
}

/// 一段的归约产物（per buff per phase）。
#[derive(Debug, Clone)]
pub struct Distrib<A> {
    /// 每源 6 向累计（含 unknown 源 —— C# `_unknownAgent` 是一等键）。
    pub by_src: BTreeMap<A, Acc>,
    /// presence_ms（by=null；任意栈在场时长）。
    pub presence_ms: i64,
    /// presence by src（该源 ≥1 栈在场时长）。
    pub presence_by: BTreeMap<A, i64>,
}

impl<A: Copy + Ord + PartialEq> Distrib<A> {
    fn entry(&mut self, k: A) -> &mut Acc {
        self.by_src.entry(k).or_default()
    }
    fn add_value(&mut self, k: A, v: i64) {
        self.entry(k).value += v;
    }
    fn add_overstack(&mut self, k: A, v: i64) {
        self.entry(k).overstack += v;
    }
    fn add_waste(&mut self, k: A, v: i64) {
        self.entry(k).waste += v;
    }
    fn add_unknown(&mut self, k: A, v: i64) {
        self.entry(k).unknown_extension += v;
    }
    fn add_extension(&mut self, k: A, v: i64) {
        self.entry(k).extension += v;
    }
    fn add_extended(&mut self, k: A, v: i64) {
        self.entry(k).extended += v;
    }

    /// GetGeneration(buffID, src) / GetOverstack / ... 存取面。
    pub fn generation_of(&self, k: A) -> i64 {
        self.by_src.get(&k).map(|a| a.value).unwrap_or(0)
    }
    pub fn overstack_of(&self, k: A) -> i64 {
        self.by_src.get(&k).map(|a| a.overstack).unwrap_or(0)
    }
    pub fn waste_of(&self, k: A) -> i64 {
        self.by_src.get(&k).map(|a| a.waste).unwrap_or(0)
    }
    pub fn unknown_ext_of(&self, k: A) -> i64 {
        self.by_src.get(&k).map(|a| a.unknown_extension).unwrap_or(0)
    }
    pub fn extension_of(&self, k: A) -> i64 {
        self.by_src.get(&k).map(|a| a.extension).unwrap_or(0)
    }
    pub fn extended_of(&self, k: A) -> i64 {
        self.by_src.get(&k).map(|a| a.extended).unwrap_or(0)
    }
    pub fn presence_of(&self, k: A) -> i64 {
        self.presence_by.get(&k).copied().unwrap_or(0)
    }
}

/// ComputeBuffDistribution + ComputeBuffPresence 的 phase 归约
/// （BuffDistribution 字典逐段累加；每段每组的 attrib 规则同
/// BuffSimulationItemBase 变体族 —— 归组 key 已含 (src,seed,is_ext)，组内
/// count 为 weight）。
///
/// 说明：C# 用 `Englobing` 变体把值分摊到 englobing 根的段上；Rust 按整
/// agent id 记账（P3a 样例无 split/englobing —— 记有意偏差，见 design.md）。
pub fn phase_distrib<A: AgentKey>(
    res: &SimResult<A>,
    start: i64,
    end: i64,
) -> Distrib<A> {
    let mut out = Distrib {
        by_src: BTreeMap::new(),
        presence_ms: 0,
        presence_by: BTreeMap::new(),
    };
    for item in &res.items {
        let c_dur = clamped(start, end, item.start, item.end);
        if c_dur <= 0 {
            continue;
        }
        out.presence_ms += c_dur;
        for g in &item.groups {
            // 每 group：主时长 → g.src；isExtension → extension → g.src；
            // seed != src（身份不等）→ extended → seed（+src unknown 时
            // unknownExt → seed）。weight = count × cDur。
            let w = c_dur * g.count;
            out.add_value(g.src, w);
            let has_seed = g.seed_src != g.src;
            if has_seed {
                out.add_extended(g.seed_src, w);
                if g.src == A::unknown() {
                    out.add_unknown(g.seed_src, w);
                }
            }
            if g.is_extension {
                out.add_extension(g.src, w);
            }
        }
        // presence by src（该源 ≥1 栈在场 —— 每 item 每源至多一次；
        // C# GetClampedDuration(actor) 按 item 判 GetActiveStacks(actor)>0）
        let mut seen: Vec<A> = Vec::new();
        for g in &item.groups {
            if !seen.contains(&g.src) {
                seen.push(g.src);
            }
        }
        for src in seen {
            *out.presence_by.entry(src).or_default() += c_dur;
        }
    }
    for w in &res.wastes {
        if start <= w.time && w.time <= end {
            out.add_waste(w.src, w.value);
        }
    }
    for w in &res.overstacks {
        if start <= w.time && w.time <= end {
            out.add_overstack(w.src, w.value);
        }
    }
    out
}

/// `Math.Round(x, 3)`（C# 默认 ToEven = 银行家舍入；ParserHelper.BuffDigit=3）。
pub fn round3(x: f64) -> f64 {
    (x * 1000.0).round_ties_even() / 1000.0
}

/// BuffStatistics.GetBuffsForSelf 的 per-phase 结果
/// （BuffStatistics.cs:161-251）—— 主数字（uptime 系）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SelfStats {
    pub uptime: f64,
    pub presence: f64,
    pub generation: f64,
    pub generation_presence: f64,
    pub overstack: f64,
    pub wasted: f64,
    pub unknown_extended: f64,
    pub by_extension: f64,
    pub extended: f64,
}

/// `GetBuffsForSelf`（BuffStatistics.cs:161-251）：查询 actor 自身分布上的
/// buff 统计。返回 (stats, statsActive)。
#[allow(clippy::too_many_arguments)]
pub fn self_stats(
    intensity: bool,
    distrib: &Distrib<u32>,
    presence_ms: i64,
    presence_by_self: i64,
    phase_duration: i64,
    active_duration: i64,
    self_key: u32,
) -> (SelfStats, SelfStats) {
    let mut s = SelfStats::default();
    let mut sa = SelfStats::default();
    let phase_d = phase_duration as f64;
    let active_d = active_duration as f64;
    let generation_value = distrib.generation_of(self_key) as f64;
    let uptime_value = total_value(&distrib.by_src) as f64;
    let overstack_value = distrib.overstack_of(self_key) as f64;
    let waste_value = distrib.waste_of(self_key) as f64;
    let unknown_ext_value = distrib.unknown_ext_of(self_key) as f64;
    let extension_value = distrib.extension_of(self_key) as f64;
    let extended_value = distrib.extended_of(self_key) as f64;
    if !intensity {
        // :187-207（duration：×100；无 presence 分支）
        s.uptime = round3(100.0 * uptime_value / phase_d);
        s.generation = round3(100.0 * generation_value / phase_d);
        s.overstack = round3(100.0 * (overstack_value + generation_value) / phase_d);
        s.wasted = round3(100.0 * waste_value / phase_d);
        s.unknown_extended = round3(100.0 * unknown_ext_value / phase_d);
        s.by_extension = round3(100.0 * extension_value / phase_d);
        s.extended = round3(100.0 * extended_value / phase_d);
        if active_d > 0.0 {
            sa.uptime = round3(100.0 * uptime_value / active_d);
            sa.generation = round3(100.0 * generation_value / active_d);
            sa.overstack = round3(100.0 * (overstack_value + generation_value) / active_d);
            sa.wasted = round3(100.0 * waste_value / active_d);
            sa.unknown_extended = round3(100.0 * unknown_ext_value / active_d);
            sa.by_extension = round3(100.0 * extension_value / active_d);
            sa.extended = round3(100.0 * extended_value / active_d);
        }
    } else {
        // :208-247（intensity：无 ×100 的 uptime；presence 分支）
        s.uptime = round3(uptime_value / phase_d);
        s.generation = round3(generation_value / phase_d);
        s.overstack = round3((overstack_value + generation_value) / phase_d);
        s.wasted = round3(waste_value / phase_d);
        s.unknown_extended = round3(unknown_ext_value / phase_d);
        s.by_extension = round3(extension_value / phase_d);
        s.extended = round3(extended_value / phase_d);
        if active_d > 0.0 {
            sa.uptime = round3(uptime_value / active_d);
            sa.generation = round3(generation_value / active_d);
            sa.overstack = round3((overstack_value + generation_value) / active_d);
            sa.wasted = round3(waste_value / active_d);
            sa.unknown_extended = round3(unknown_ext_value / active_d);
            sa.by_extension = round3(extension_value / active_d);
            sa.extended = round3(extended_value / active_d);
        }
        s.presence = round3(100.0 * presence_ms as f64 / phase_d);
        if active_d > 0.0 {
            sa.presence = round3(100.0 * presence_ms as f64 / active_d);
        }
        s.generation_presence = round3(100.0 * presence_by_self as f64 / phase_d);
        if active_d > 0.0 {
            sa.generation_presence = round3(100.0 * presence_by_self as f64 / active_d);
        }
    }
    (s, sa)
}

/// BuffByActorStatistics.GetBuffByActor（BuffByActorStatistics.cs:20-83）——
/// per-src 7 向（rate 化）。`srcs` = 分布内出现过的源；duration 型乘 100；
/// intensity 的 generatedPresence = 100 × presence_by[src]。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PerSrcStats {
    pub generated: f64,
    pub generated_presence: f64,
    pub overstacked: f64,
    pub wasted: f64,
    pub unknown_extended: f64,
    pub by_extension: f64,
    pub extended: f64,
}

/// `GetBuffByActor` 完整（BuffByActorStatistics.cs:20-83）：分母 phase 用
/// `phase_duration`、active 用 `active_duration`（≤0 时 active 侧全 0）。
/// presence_by 仅 intensity 的 generatedPresence 用。
#[allow(clippy::too_many_arguments)]
pub fn by_actor(
    intensity: bool,
    d: &Distrib<u32>,
    srcs: &[u32],
    presence_by: &BTreeMap<u32, i64>,
    phase_duration: i64,
    active_duration: i64,
) -> (BTreeMap<u32, PerSrcStats>, BTreeMap<u32, PerSrcStats>) {
    let rate = |src: u32, denom: i64| -> PerSrcStats {
        let denom_f = denom as f64;
        let genv = d.generation_of(src) as f64;
        let overstacked = d.overstack_of(src) as f64 + genv;
        let wasted = d.waste_of(src) as f64;
        let unknown_ext = d.unknown_ext_of(src) as f64;
        let extension = d.extension_of(src) as f64;
        let extended = d.extended_of(src) as f64;
        let mut r = PerSrcStats::default();
        if denom_f > 0.0 {
            if !intensity {
                r.generated = round3(genv * 100.0 / denom_f);
                r.overstacked = round3(overstacked * 100.0 / denom_f);
                r.wasted = round3(wasted * 100.0 / denom_f);
                r.unknown_extended = round3(unknown_ext * 100.0 / denom_f);
                r.by_extension = round3(extension * 100.0 / denom_f);
                r.extended = round3(extended * 100.0 / denom_f);
            } else {
                r.generated = round3(genv / denom_f);
                r.overstacked = round3(overstacked / denom_f);
                r.wasted = round3(wasted / denom_f);
                r.unknown_extended = round3(unknown_ext / denom_f);
                r.by_extension = round3(extension / denom_f);
                r.extended = round3(extended / denom_f);
            }
            if intensity {
                let p = presence_by.get(&src).copied().unwrap_or(0);
                r.generated_presence = round3(100.0 * p as f64 / denom_f);
            }
        }
        r
    };
    let mut rates = BTreeMap::new();
    let mut rates_active = BTreeMap::new();
    for &src in srcs {
        rates.insert(src, rate(src, phase_duration));
        rates_active.insert(src, rate(src, active_duration));
    }
    (rates, rates_active)
}

/// 分布键列表（C# GetSrcs 语义的键集合）。
pub fn srcs_of<A: Copy + Ord>(d: &Distrib<A>) -> Vec<A> {
    d.by_src.keys().copied().collect()
}

/// 段装配 + 融合 —— C# 图构建的精确复刻（SingleActorBuffsHelper 逐 item
/// 补零装配 → StateGraph.FuseSegments + Segment.FuseConsecutive）：
/// 1. 装配：首段前补 [logStart, firstStart]（可与首段同刻 → 零长头段）、
///    段间补缝、尾部补 [lastEnd, logEnd]（可与尾段同刻 → 零长尾段）。
/// 2. FuseSegments：剔除 Start>End。
/// 3. FuseConsecutive：**中间**零长段跳过；索引 0 的零长头段保留
///    （C# 循环从 i=1 起，头段从不被检查）；尾段为最后一个元素恒写入
///    （零长尾段会先与同值前段合并，异值时保留？—— C# 尾段（零长）在
///    循环中被 continue 跳过（i>0），不入 last —— 见下实现）。
///
/// C# FuseConsecutive 的行为细节：
/// - `last=segs[0]`，从 i=1 遍历：`cur.IsEmpty()` → continue（零长中段
///   丢弃）；与 last 同值 → 并入 last（last.End=cur.End）；异值 →
///   segs[lastIndex]=last；last=cur。
/// - 循环后 `segs[lastIndex++]=last` 且 RemoveRange 尾部 —— 因此**零长
///   头段（索引 0，从未被当作 cur 处理）保留**；零长尾段（i>0 时被
///   continue）丢弃；若整个列表只有一段则原样保留。
pub fn fuse_segments(
    log_start: i64,
    log_end: i64,
    segs: Vec<(i64, i64, i64)>,
) -> Vec<(i64, i64, i64)> {
    // 装配（等价 C# graphSegments 装配）
    let mut out: Vec<(i64, i64, i64)> = Vec::new();
    for (s, e, v) in segs {
        match out.last() {
            None => {
                out.push((log_start, s, 0));
            }
            Some(&(_, pe, _)) => {
                if pe != s {
                    out.push((pe, s, 0));
                }
            }
        }
        out.push((s, e, v));
    }
    if out.is_empty() {
        out.push((log_start, log_end, 0));
    } else {
        let le = out.last().expect("e").1;
        out.push((le, log_end, 0));
    }
    // FuseSegments：Start > End 剔除（不可能出现，防御）
    out.retain(|&(s, e, _)| s <= e);
    // FuseConsecutive（Segment.cs:17-46 逐行复刻）
    if out.is_empty() {
        return out;
    }
    let mut last = out[0];
    let mut last_index = 0usize;
    let mut i = 1usize;
    while i < out.len() {
        let cur = out[i];
        if cur.0 >= cur.1 {
            // IsEmpty → 中间零长段丢弃（头段在 i=0 从未检查；尾段在此丢弃）
            i += 1;
            continue;
        }
        if cur.2 == last.2 {
            last.1 = cur.1; // last.End = cur.End
            out[last_index] = last;
        } else {
            out[last_index] = last;
            last = cur;
            last_index += 1;
        }
        i += 1;
    }
    out[last_index] = last;
    out.truncate(last_index + 1);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{AgentKey, SimCmd, Simulator};
    use gw2ei_parse::BuffStackType as B;

    fn no_heal(_: u32) -> u16 {
        0
    }
    fn run(events: Vec<SimCmd<u32>>, stack_type: B, capacity: i64, log_end: i64) -> crate::sim::SimResult<u32> {
        let mut sim = Simulator::new(stack_type, capacity, &no_heal);
        sim.simulate(&events, 0, log_end);
        sim.result
    }

    fn add(dur: i64, src: u32, start: i64) -> SimCmd<u32> {
        SimCmd::Add { duration: dur, src, start, stack_id: 0, added_active: false, overriden_duration: 0, overriden_stack_id: 0 }
    }

    #[test]
    fn duration_value_and_presence() {
        let r = run(vec![add(4_000, 1, 0)], B::Force, 1, 10_000);
        let d = phase_distrib(&r, 1_000, 9_000);
        assert_eq!(d.by_src.get(&1).map(|a| a.value), Some(3_000));
        assert_eq!(d.presence_ms, 3_000);
        assert_eq!(d.presence_by.get(&1), Some(&3_000));
        assert_eq!(d.presence_by.get(&2), None);
        let d0 = phase_distrib(&r, 5_000, 9_000);
        assert_eq!(total_value(&d0.by_src), 0);
        let d1 = phase_distrib(&r, 3_500, 4_500);
        assert_eq!(d1.presence_ms, 500);
    }

    #[test]
    fn intensity_multi_stack_value() {
        let r = run(
            vec![add(4_000, 1, 0), add(4_000, 2, 2_000)],
            B::Stacking,
            5,
            10_000,
        );
        let d = phase_distrib(&r, 0, 10_000);
        // 两栈各全程 4s：value=8_000；在场并集 [0..6_000]=6_000
        assert_eq!(total_value(&d.by_src), 8_000);
        assert_eq!(d.presence_ms, 6_000);
        assert_eq!(d.presence_by.get(&1), Some(&4_000));
        assert_eq!(d.presence_by.get(&2), Some(&4_000));
    }

    #[test]
    fn waste_inclusive_bounds() {
        let r = run(vec![add(1_000, 1, 0)], B::Force, 1, 10_000);
        let mut rr = crate::sim::SimResult {
            items: Vec::new(),
            wastes: Vec::new(),
            overstacks: Vec::new(),
        };
        rr.overstacks.push(crate::sim::WasteEvent { time: 5_000, src: 1, value: 100 });
        rr.wastes.push(crate::sim::WasteEvent { time: 5_000, src: 2, value: 50 });
        let d = phase_distrib(&rr, 5_000, 6_000);
        assert_eq!(d.overstack_of(1), 100);
        assert_eq!(d.waste_of(2), 50);
        let d2 = phase_distrib(&rr, 5_001, 6_000);
        assert_eq!(d2.overstack_of(1), 0);
        let _ = r;
    }

    #[test]
    fn unknown_src_keys() {
        let r = run(vec![add(1_000, u32::unknown(), 0)], B::Force, 1, 10_000);
        let d = phase_distrib(&r, 0, 10_000);
        assert_eq!(d.generation_of(u32::unknown()), 1_000);
    }

    #[test]
    fn round3_banker() {
        assert_eq!(round3(1.2345), 1.234); // ties-to-even
        assert_eq!(round3(1.2355), 1.236);
        assert_eq!(round3(2.5), 2.5);
    }

    #[test]
    fn fuse_zero_fill_and_merge() {
        let fused = fuse_segments(0, 10, vec![(2, 3, 1), (3, 5, 2)]);
        assert_eq!(fused, vec![(0, 2, 0), (2, 3, 1), (3, 5, 2), (5, 10, 0)]);
        let fused2 = fuse_segments(0, 10, vec![(0, 2, 1), (2, 2, 1), (2, 4, 1)]);
        // 首段与 logStart 同刻 → 装配出零长头段，C# FuseConsecutive 从头段
        // 之后遍历 → 头段保留（states 里表现为 [0,0],[0,v] 双条目）
        assert_eq!(fused2, vec![(0, 0, 0), (0, 4, 1), (4, 10, 0)]);
        // 首段迟于 logStart → 正长头段
        let fused3 = fuse_segments(0, 10, vec![(5, 8, 1)]);
        assert_eq!(fused3, vec![(0, 5, 0), (5, 8, 1), (8, 10, 0)]);
    }
}
