//! Buff 堆叠仿真引擎 —— EI `BuffSimulators/NoID` 家族的忠实移植
//! （AbstractBuffSimulator.cs + BuffSimulator.cs + BuffSimulatorDuration.cs +
//! BuffSimulatorIntensity.cs + EffectStackingLogic/{Queue,ForceOverride,Override,
//! Healing}Logic.cs + BuffStackItem.cs）。
//!
//! 语义锚点：
//! - 每 (actor, buff) 一个仿真器实例，喂入**已清洗、时间稳定升序**的事件
//!   序列（清洗管线见 `pipeline` 模块；同 ms 保 evtc 序 —— SortStable）。
//! - `CombatData.cs:618 UseBuffInstanceSimulator=false` 写死 → 只移植 NoID
//!   家族；BuffSimulatorID* 是死代码不复刻。
//! - 引擎对 agent 泛型 `A`（调用方用 `AgentId`；unknown 用 `u32::MAX`）。
//! - C# `BuffStackItemPool` 是对象池（无语义影响）不复刻；C# 用对象引用
//!   定位队列 Activate 的栈 → Rust 用自增 `uid`（见 `StackItem.uid`）。
//! - C# `RegroupedStack`（BuffSimulationItemIntensity）按
//!   (Src.InstID, SeedSrc.InstID, IsExtension) 归组 —— 归组是纯优化
//!   （item end ≤ 各栈余量，同组逐栈记账结果一致）；Rust 按完整三元组
//!   (src, seed, is_ext) 归组（不合并 instid 相同的不同 agent —— 差异仅
//!   影响 C# instid 碰撞场景，见 design.md 有意偏差登记）。
//!
//! 输出（`SimResult`）供统计层消费：`items`（GenerationSimulation 段）、
//! `wastes`/`overstacks`（废弃/溢出事件）。
//!
//! 魔法常量：`REMOVE_TOLERANCE=15`（ParserHelper.BuffSimulatorDelayConstant）。

/// C# `ParserHelper.BuffSimulatorDelayConstant` = 15ms（RemoveSingle 近似匹配）。
pub const REMOVE_TOLERANCE: i64 = 15;

/// C# `ParserHelper.BuffSimulatorStackActiveDelayConstant`（HealingLogic
/// Activate 的 50ms 保鲜判断 —— HealingLogic.cs:63 的 `TotalDuration < 50`）。
pub const STACK_ACTIVE_FRESH: i64 = 50;

/// agent 泛型键（unknown 用 `u32::MAX` —— C# `_unknownAgent`）。
pub type Agent = u32;
pub const UNKNOWN_AGENT: Agent = u32::MAX;

/// 泛型键的 unknown 常量访问（A 需自带 unknown —— 调用方用 AgentId）。
pub trait AgentKey: Copy + PartialEq + Ord {
    fn unknown() -> Self;
}

/// `AgentId` 语义：unknown = `u32::MAX`（gw2ei-model `NO_AGENT`）。
impl AgentKey for u32 {
    fn unknown() -> Self {
        u32::MAX
    }
}

/// `Buff.Type`（Buff.cs:45-56）：Queue/Regeneration/Force → Duration；
/// Stacking×3 → Intensity。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuffType {
    Duration,
    Intensity,
}

impl BuffType {
    pub fn of(stack_type: gw2ei_parse::BuffStackType) -> BuffType {
        match stack_type {
            gw2ei_parse::BuffStackType::Stacking
            | gw2ei_parse::BuffStackType::StackingUniquePerSrc
            | gw2ei_parse::BuffStackType::StackingConditionalLoss => BuffType::Intensity,
            gw2ei_parse::BuffStackType::Queue
            | gw2ei_parse::BuffStackType::Regeneration
            | gw2ei_parse::BuffStackType::Force => BuffType::Duration,
            gw2ei_parse::BuffStackType::Unknown => panic!("Buffs can not be typless"), // BuffSimulator.cs:44
        }
    }
}

/// BuffStackItem（BuffStackItem.cs:5-77）。
#[derive(Debug, Clone)]
pub struct StackItem<A> {
    /// 对象身份（C# 引用等价物；队列 Activate 定位用）。
    pub uid: u64,
    pub start: i64,
    pub duration: i64,
    pub src: A,
    pub seed_src: A,
    pub is_extension: bool,
    pub stack_id: u64,
    /// 续命队列 [(src, value)]。
    pub extensions: Vec<(A, i64)>,
}

impl<A: Copy + PartialEq> StackItem<A> {
    /// `TotalDuration`：主 + Σ 扩展。
    pub fn total_duration(&self) -> i64 {
        self.duration + self.extensions.iter().map(|(_, v)| v).sum::<i64>()
    }

    /// `Shift(startShift, durationShift)`（BuffStackItem.cs:44-54）：
    /// Start 前移、Duration 扣减；主时长耗尽且队列非空 → 队头 extension
    /// 转正（Src/Duration 替换、IsExtension=true）。C# 两次调用
    /// `Shift(0,diff)`（可转正）与 `Shift(diff,0)`（只前移）—— 合并为一次
    /// 调用语义不变（转正与 start 前移互不依赖）。
    pub fn shift(&mut self, start_shift: i64, duration_shift: i64) {
        self.start += start_shift;
        self.duration -= duration_shift;
        if self.duration == 0 && !self.extensions.is_empty() {
            let (src, value) = self.extensions.remove(0);
            self.src = src;
            self.duration = value;
            self.is_extension = true;
        }
    }
}

/// 生成段（BuffSimulationItem*）：[start, end] 窗 + 归组后的 stack 明细。
/// 窗内各组 stack 全程存活（end ≤ 最短栈余量 —— C# `OverrideEnd(start+diff)`）。
#[derive(Debug, Clone)]
pub struct GenItem<A> {
    pub start: i64,
    pub end: i64,
    /// 归组 stack。Duration 型只有头栈（GetActiveStacks()=1 —— 非头栈不进
    /// attrib/图值）。
    pub groups: Vec<Group<A>>,
}

/// 同 key stack 组（C# RegroupedStack：Item + StackCount）。
#[derive(Debug, Clone)]
pub struct Group<A> {
    pub src: A,
    pub seed_src: A,
    pub is_extension: bool,
    pub count: i64,
}

/// 废弃/溢出事件（BuffSimulationItemWasted/Overstack：时刻 + src + 值；
/// `GetValue(start,end) = start<=Time<=end ? value : 0` 含边界）。
#[derive(Debug, Clone, Copy)]
pub struct WasteEvent<A> {
    pub time: i64,
    pub src: A,
    pub value: i64,
}

/// 一次 (actor, buff) 仿真的输出。
#[derive(Debug, Clone)]
pub struct SimResult<A> {
    /// GenerationSimulation（时间有序）。
    pub items: Vec<GenItem<A>>,
    /// WasteSimulationResult。
    pub wastes: Vec<WasteEvent<A>>,
    /// OverstackSimulationResult。
    pub overstacks: Vec<WasteEvent<A>>,
}

/// 仿真命令（由 pipeline 按 BuffEvent.UpdateSimulator 语义转换；NoID
/// forceActive=true）。
#[derive(Debug, Clone, Copy)]
pub enum SimCmd<A> {
    /// BuffApplyEvent.UpdateSimulator（BuffApplyEvent.cs:36-40）：
    /// `Add(AppliedDuration, CreditedBy, Time, BuffInstance, addedActive,
    /// OverridenRegenDuration, OverridenRegenInstance)`。
    Add {
        duration: i64,
        src: A,
        start: i64,
        stack_id: u64,
        added_active: bool,
        overriden_duration: i64,
        overriden_stack_id: u64,
    },
    /// BuffExtensionEvent.UpdateSimulator：`Extend(ExtendedDuration,
    /// OldDuration, CreditedBy, Time, BuffInstance)`（修正后 <1ms 不发）。
    Extend {
        extension: i64,
        old_value: i64,
        src: A,
        time: i64,
        stack_id: u64,
    },
    /// BuffRemoveSingle/All.UpdateSimulator：
    /// `Remove(by, RemovedDuration, removedStacks, Time, Single|All, stackID)`。
    Remove {
        removed_duration: i64,
        time: i64,
        all: bool,
    },
    /// BuffStackActiveEvent.UpdateSimulator：Activate(BuffInstance)。
    Activate { time: i64, stack_id: u64 },
    /// BuffStackDeactiveEvent.UpdateSimulator：Deactivate(BuffInstance, ResetToDuration)。
    Deactivate {
        time: i64,
        stack_id: u64,
        to_duration: i64,
    },
}

impl<A> SimCmd<A> {
    pub fn time(&self) -> i64 {
        match *self {
            SimCmd::Add { start, .. } => start,
            SimCmd::Extend { time, .. }
            | SimCmd::Remove { time, .. }
            | SimCmd::Activate { time, .. }
            | SimCmd::Deactivate { time, .. } => time,
        }
    }
}

// ===== 仿真器 =====

pub struct Simulator<'a, A> {
    buff_type: BuffType,
    logic: StackLogic,
    capacity: i64,
    /// Regen 排序取 SeedSrc.Healing（调用方注入；unknown → 0）。
    healing_of: &'a dyn Fn(A) -> u16,
    next_uid: u64,
    stacks: Vec<StackItem<A>>,
    pub result: SimResult<A>,
    // BuffSimulatorDuration 单槽 / BuffSimulatorIntensity 队列
    last_src_remove: (A, bool),
    last_src_removes: Vec<(A, bool)>,
    /// HealingLogic `_noSort`（见过 stack-active 后永久停排序 —— 实例状态，
    /// 不可共享实例，Regen 每 (actor,buff) 一个仿真器）。
    no_sort: bool,
}

/// 堆叠逻辑（BuffSimulator.cs:23-45 的 switch）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StackLogic {
    Queue,
    Force,
    Override,
    /// Regeneration：Queue + healing 排序 + 实例状态 `_noSort`。
    Healing,
}

impl StackLogic {
    fn of(stack_type: gw2ei_parse::BuffStackType) -> StackLogic {
        match stack_type {
            gw2ei_parse::BuffStackType::Queue => StackLogic::Queue,
            gw2ei_parse::BuffStackType::Regeneration => StackLogic::Healing,
            gw2ei_parse::BuffStackType::Force => StackLogic::Force,
            gw2ei_parse::BuffStackType::Stacking
            | gw2ei_parse::BuffStackType::StackingUniquePerSrc
            | gw2ei_parse::BuffStackType::StackingConditionalLoss => StackLogic::Override,
            gw2ei_parse::BuffStackType::Unknown => panic!("Buffs can not be typless"), // BuffSimulator.cs:44
        }
    }
}

impl<'a, A: AgentKey> Simulator<'a, A> {
    /// `stack_type`/`capacity` 由调用方按 Buff 定义提供（BuffSimulator.cs:19-47）。
    /// `healing_of`：HealingLogic 排序键（SeedSrc.Healing）。
    pub fn new(
        stack_type: gw2ei_parse::BuffStackType,
        capacity: i64,
        healing_of: &'a dyn Fn(A) -> u16,
    ) -> Self {
        let buff_type = BuffType::of(stack_type);
        Simulator {
            buff_type,
            logic: StackLogic::of(stack_type),
            capacity,
            healing_of,
            next_uid: 0,
            stacks: Vec::new(),
            result: SimResult {
                items: Vec::new(),
                wastes: Vec::new(),
                overstacks: Vec::new(),
            },
            last_src_remove: (A::unknown(), false),
            last_src_removes: Vec::new(),
            no_sort: false,
        }
    }

    pub fn buff_type(&self) -> BuffType {
        self.buff_type
    }

    fn uid(&mut self) -> u64 {
        let u = self.next_uid;
        self.next_uid += 1;
        u
    }

    fn is_full(&self) -> bool {
        match self.logic {
            // ForceOverrideLogic.IsFull：Count == 1（其余 Count == capacity）。
            StackLogic::Force => self.stacks.len() == 1,
            _ => self.stacks.len() as i64 == self.capacity,
        }
    }

    /// 带 seed 的 Add（BuffSimulator.cs:88-92；Extend 断链续接用）。
    fn new_stack(&mut self, duration: i64, src: A, start: i64, stack_id: u64) -> StackItem<A> {
        StackItem {
            uid: self.uid(),
            start,
            duration,
            src,
            seed_src: src,
            is_extension: false,
            stack_id,
            extensions: Vec::new(),
        }
    }

    fn new_stack_seeded(
        &mut self,
        duration: i64,
        src: A,
        seed_src: A,
        start: i64,
        is_extension: bool,
        stack_id: u64,
    ) -> StackItem<A> {
        StackItem {
            uid: self.uid(),
            start,
            duration,
            src,
            seed_src,
            is_extension,
            stack_id,
            extensions: Vec::new(),
        }
    }

    // ===== 仿真主循环（AbstractBuffSimulator.cs:44-67）=====

    /// `Simulate(events, logStart, logEnd)`：一次性。事件须已时间升序
    /// （同 ms 保 evtc 序）。Simulate 后清洗层不再喂命令。
    pub fn simulate(&mut self, events: &[SimCmd<A>], log_start: i64, log_end: i64) {
        if !self.result.items.is_empty() {
            return;
        }
        let mut time_prev = if events.is_empty() {
            log_start
        } else {
            events[0].time().min(log_start)
        };
        for &e in events {
            let time_cur = e.time();
            debug_assert!(time_cur >= time_prev, "Negative passed time in boon simulation");
            self.update(time_cur - time_prev);
            self.apply_cmd(e);
            time_prev = time_cur;
        }
        self.update(log_end - time_prev);
        self.trim(log_end);
        // GenerationSimulation.RemoveAll(x => x.Duration <= 0)
        self.result.items.retain(|i| i.end - i.start > 0);
    }

    fn apply_cmd(&mut self, cmd: SimCmd<A>) {
        match cmd {
            SimCmd::Add {
                duration,
                src,
                start,
                stack_id,
                added_active,
                overriden_duration,
                overriden_stack_id,
            } => {
                let to_add = self.new_stack(duration, src, start, stack_id);
                self.add_inner(to_add, added_active, overriden_duration, overriden_stack_id);
            }
            SimCmd::Extend {
                extension,
                old_value,
                src,
                time,
                stack_id,
            } => self.extend(extension, old_value, src, time, stack_id),
            SimCmd::Remove {
                removed_duration,
                time,
                all,
            } => {
                if all {
                    self.remove_all(time);
                } else {
                    self.remove_single(removed_duration, time);
                }
            }
            SimCmd::Activate { stack_id, .. } => self.activate(stack_id),
            SimCmd::Deactivate {
                stack_id, to_duration, ..
            } => self.deactivate(stack_id, to_duration),
        }
    }

    // ===== 逻辑层（EffectStackingLogic/*.cs）=====

    /// Add + Sort。Queue/Force 无排序；Override 二分升序插；Healing 按
    /// healing 降序（_noSort 停排序）。
    fn add_by_logic(&mut self, to_add: StackItem<A>) {
        match self.logic {
            StackLogic::Queue | StackLogic::Force => self.stacks.push(to_add),
            StackLogic::Healing => {
                self.stacks.push(to_add);
                if !self.no_sort {
                    // Healing 降序；同值保持插入序（.NET List.Sort 小列表走
                    // 稳定插入排序路径；Rust sort_by_key 稳定排序一致）
                    self.stacks
                        .sort_by_key(|x| std::cmp::Reverse((self.healing_of)(x.seed_src)));
                }
            }
            StackLogic::Override => {
                if self.stacks.is_empty() {
                    // OverrideLogic.Add：Count == 0 直接 Add
                    self.stacks.push(to_add);
                } else {
                    let dur = to_add.total_duration();
                    let idx = self.override_insert_index(dur, 0, self.stacks.len() - 1);
                    if idx > self.stacks.len() - 1 {
                        self.stacks.push(to_add);
                    } else {
                        self.stacks.insert(idx, to_add);
                    }
                }
            }
        }
    }

    /// OverrideLogic.BinarySearchBuffStackItem（升序；相等插后）。
    fn override_insert_index(
        &self,
        total_duration: i64,
        min_index: usize,
        max_index: usize,
    ) -> usize {
        let stacks = &self.stacks;
        if stacks[min_index].total_duration() > total_duration {
            return min_index;
        }
        if stacks[max_index].total_duration() < total_duration {
            return max_index + 1;
        }
        if min_index > max_index {
            return min_index;
        }
        let mid_index = (min_index + max_index) / 2;
        if total_duration == stacks[mid_index].total_duration() {
            mid_index + 1
        } else if total_duration < stacks[mid_index].total_duration() {
            self.override_insert_index(total_duration, min_index, mid_index.saturating_sub(1))
        } else {
            self.override_insert_index(total_duration, mid_index + 1, max_index)
        }
    }

    /// BuffSimulator.Add 私有入口（:57-76）：空位 Add；满员 FindLowestValue
    /// （false → Overstack 记录）；addedActive → Activate 到 0 位。
    fn add_inner(
        &mut self,
        to_add: StackItem<A>,
        added_active: bool,
        overriden_duration: i64,
        overriden_stack_id: u64,
    ) {
        let uid = to_add.uid;
        let (start, duration, src) = (to_add.start, to_add.duration, to_add.src);
        let extensions = to_add.extensions.clone();
        let (seed, is_ext) = (to_add.seed_src, to_add.is_extension);
        if !self.is_full() {
            self.add_by_logic(to_add);
        } else if !self.find_lowest_value(
            uid,
            duration,
            src,
            start,
            &extensions,
            seed,
            is_ext,
            overriden_duration,
            overriden_stack_id,
        ) {
            // BuffSimulationItemOverstack(toAdd.Src, toAdd.Duration, toAdd.Start)
            self.result.overstacks.push(WasteEvent {
                time: start,
                src,
                value: duration,
            });
        }
        if added_active {
            self.activate_uid(uid);
        }
    }

    /// FindLowestValue：淘汰 + 入位（true）；无处安放 false → overstack。
    /// overriden_*：Healing 专用（AddRegen 的 OverridenDuration/Instance）。
    #[allow(clippy::too_many_arguments)]
    fn find_lowest_value(
        &mut self,
        uid: u64,
        duration: i64,
        src: A,
        start: i64,
        extensions: &[(A, i64)],
        seed: A,
        is_ext: bool,
        overriden_duration: i64,
        overriden_stack_id: u64,
    ) -> bool {
        let remove_idx = match self.logic {
            StackLogic::Queue => {
                // QueueLogic.cs:13-17：非头中 TotalDuration 最小
                if self.stacks.len() <= 1 {
                    panic!("Queue logic based must have a > 1 capacity");
                }
                (1..self.stacks.len())
                    .min_by_key(|&i| self.stacks[i].total_duration())
                    .expect("len > 1")
            }
            StackLogic::Force => {
                if self.stacks.is_empty() {
                    return false;
                }
                0
            }
            StackLogic::Override => {
                if self.stacks.is_empty() {
                    return false;
                }
                0
            }
            StackLogic::Healing => {
                // HealingLogic.cs:39-48：overridenStackID → |总时长−overriden|
                // 近似 → 末位（"old school pre stack instance" 排序时代）
                if self.stacks.len() <= 1 {
                    panic!("Queue logic based must have a > 1 capacity");
                }
                let by_id = if overriden_stack_id > 0 {
                    self.stacks.iter().position(|s| s.stack_id == overriden_stack_id)
                } else {
                    None
                };
                let by_dur = by_id.or_else(|| {
                    if overriden_duration > 0 {
                        self.stacks
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, s)| (s.total_duration() - overriden_duration).abs())
                            .map(|(i, _)| i)
                    } else {
                        None
                    }
                });
                by_dur.unwrap_or(self.stacks.len() - 1)
            }
        };
        let to_remove = self.stacks.remove(remove_idx);
        self.push_waste(&to_remove, to_remove.start);
        let to_add = StackItem {
            uid,
            start,
            duration,
            src,
            seed_src: seed,
            is_extension: is_ext,
            stack_id: 0,
            extensions: extensions.to_vec(),
        };
        match self.logic {
            StackLogic::Override => self.add_by_logic(to_add),
            _ => {
                self.stacks.insert(remove_idx, to_add);
            }
        }
        true
    }

    /// 整栈废弃记录（淘汰：时间 = 被淘汰栈 Start；移除：调用方传移除时刻）。
    /// 主时长 + 每笔 Extension 各一条（QueueLogic/ForceOverride/Override/
    /// Healing/Remove 全路径同构）。
    fn push_waste(&mut self, to_remove: &StackItem<A>, time: i64) {
        self.result.wastes.push(WasteEvent {
            time,
            src: to_remove.src,
            value: to_remove.duration,
        });
        for &(src, value) in &to_remove.extensions {
            self.result.wastes.push(WasteEvent { time, src, value });
        }
    }

    /// QueueLogic/HealingLogic `Activate(stacks, stackItem)`：移到 0 位。
    ///（Override/Force 为 StackingLogic 默认 no-op。）
    fn activate_uid(&mut self, uid: u64) {
        if !matches!(self.logic, StackLogic::Queue | StackLogic::Healing) {
            return;
        }
        let Some(idx) = self.stacks.iter().position(|s| s.uid == uid) else {
            return;
        };
        let item = self.stacks.remove(idx);
        self.stacks.insert(0, item);
    }

    /// HealingLogic.Activate(stacks, stackID)（:57-80）：`_noSort=true`；
    /// 头栈总时长 < 50ms 时直接替换头栈，否则插 0 位。
    fn activate(&mut self, stack_id: u64) {
        if !matches!(self.logic, StackLogic::Healing) {
            return;
        }
        let Some(idx) = self.stacks.iter().position(|s| s.stack_id == stack_id) else {
            return;
        };
        self.no_sort = true;
        let to_activate = self.stacks.remove(idx);
        if !self.stacks.is_empty() && self.stacks[0].total_duration() < STACK_ACTIVE_FRESH {
            self.stacks[0] = to_activate;
        } else {
            self.stacks.insert(0, to_activate);
        }
    }

    /// StackingLogic.Reset 默认 no-op（NoID 下 Deactive 不进仿真 ——
    /// BuffStackDeactiveEvent.IsBuffSimulatorCompliant=false）。
    fn deactivate(&mut self, _stack_id: u64, _to_duration: i64) {}

    // ===== Remove（BuffSimulator.cs:94-136）=====

    /// Remove(All)：全栈废弃（含 Extension 逐笔），清空。
    fn remove_all(&mut self, time: i64) {
        for s in &self.stacks {
            self.result.wastes.push(WasteEvent {
                time,
                src: s.src,
                value: s.duration,
            });
            for &(src, value) in &s.extensions {
                self.result.wastes.push(WasteEvent { time, src, value });
            }
        }
        self.stacks.clear();
    }

    /// Remove(Single)：|removedDuration − TotalDuration| < 15ms 近似匹配。
    fn remove_single(&mut self, removed_duration: i64, time: i64) {
        let Some(idx) = self
            .stacks
            .iter()
            .position(|s| (removed_duration - s.total_duration()).abs() < REMOVE_TOLERANCE)
        else {
            return;
        };
        let s = self.stacks.remove(idx);
        self.push_waste(&s, time);
    }

    // ===== Extend（断链续接）=====

    /// Duration（BuffSimulatorDuration.cs:13-23）：
    /// `(Count!=0 && oldValue>0) || IsFull` → 头栈队列追加；否则按
    /// _lastSrcRemove 重建栈（Add 带 seed/extension 标记）。
    fn extend(&mut self, extension: i64, old_value: i64, src: A, start: i64, stack_id: u64) {
        match self.buff_type {
            BuffType::Duration => {
                if (!self.stacks.is_empty() && old_value > 0) || self.is_full() {
                    if let Some(head) = self.stacks.first_mut() {
                        head.extensions.push((src, extension));
                    }
                } else {
                    let (seed, is_ext) = self.last_src_remove;
                    let to_add =
                        self.new_stack_seeded(old_value + extension, src, seed, start, is_ext, stack_id);
                    self.add_inner(to_add, true, 0, 0);
                }
            }
            BuffType::Intensity => {
                // BuffSimulatorIntensity.cs:13-35
                if (!self.stacks.is_empty() && old_value > 0) || self.is_full() {
                    if let Some(s) = self
                        .stacks
                        .iter_mut()
                        .min_by_key(|s| (s.total_duration() - old_value).abs())
                    {
                        s.extensions.push((src, extension));
                    }
                } else if !self.last_src_removes.is_empty() {
                    let (seed, is_ext) = self.last_src_removes.remove(0);
                    let to_add =
                        self.new_stack_seeded(old_value + extension, src, seed, start, is_ext, stack_id);
                    self.add_inner(to_add, false, 0, 0);
                } else {
                    let to_add = self.new_stack(old_value + extension, src, start, stack_id);
                    self.add_inner(to_add, true, 0, 0);
                }
            }
        }
    }

    // ===== 时间推进（Update + Trim）=====

    fn update(&mut self, time_passed: i64) {
        match self.buff_type {
            BuffType::Duration => self.update_duration(time_passed),
            BuffType::Intensity => self.update_intensity(time_passed),
        }
    }

    fn update_duration(&mut self, time_passed: i64) {
        // BuffSimulatorDuration.Update（:27-60）：段 = 头栈剩余时长；到期
        // 转正/出队后 leftOver 递归。
        if self.stacks.is_empty() || time_passed <= 0 {
            return;
        }
        // _lastSrcRemove 每次 Update 开头复位（unknown,false）
        self.last_src_remove = (A::unknown(), false);
        let head = &self.stacks[0];
        let start = head.start;
        let time_diff = head.duration - time_passed;
        let (diff, leftover) = if time_diff < 0 {
            (head.duration, time_passed - head.duration)
        } else {
            (time_passed, 0)
        };
        // 段快照（移位前）：头栈 attrib 组
        self.result.items.push(GenItem {
            start,
            end: start + diff,
            groups: vec![Group {
                src: head.src,
                seed_src: head.seed_src,
                is_extension: head.is_extension,
                count: 1,
            }],
        });
        // activeStack.Shift(0, diff)（可转正）+ 全栈 Shift(diff, 0)
        self.stacks[0].shift(0, diff);
        for s in &mut self.stacks {
            s.shift(diff, 0);
        }
        if self.stacks[0].duration == 0 {
            // 无 extension 可转正 → 主时长耗尽出队
            self.last_src_remove = (self.stacks[0].seed_src, self.stacks[0].is_extension);
            self.stacks.remove(0);
        }
        if leftover > 0 {
            self.update_duration(leftover);
        }
    }

    fn update_intensity(&mut self, time_passed: i64) {
        // BuffSimulatorIntensity.Update（:39-71）：全栈同减最短余量；到期者
        // 记录 _lastSrcRemoves（leftOver==0 时）并移除；leftOver 递归。
        if self.stacks.is_empty() || time_passed <= 0 {
            return;
        }
        let start = self.stacks[0].start;
        let mut groups: Vec<Group<A>> = Vec::new();
        for s in &self.stacks {
            match groups
                .iter_mut()
                .find(|g| g.src == s.src && g.seed_src == s.seed_src && g.is_extension == s.is_extension)
            {
                Some(g) => g.count += 1,
                None => groups.push(Group {
                    src: s.src,
                    seed_src: s.seed_src,
                    is_extension: s.is_extension,
                    count: 1,
                }),
            }
        }
        let diff = self
            .stacks
            .iter()
            .map(|s| s.duration)
            .min()
            .unwrap_or(0)
            .min(time_passed);
        let leftover = time_passed - diff;
        self.result.items.push(GenItem {
            start,
            end: start + diff,
            groups,
        });
        if leftover == 0 {
            self.last_src_removes.clear();
        }
        let mut expired: Vec<usize> = Vec::new();
        for (i, s) in self.stacks.iter_mut().enumerate() {
            s.shift(diff, diff);
            if s.duration == 0 {
                expired.push(i);
            }
        }
        for &i in expired.iter().rev() {
            if leftover == 0 {
                let s = &self.stacks[i];
                self.last_src_removes.push((s.seed_src, s.is_extension));
            }
            self.stacks.remove(i);
        }
        if leftover > 0 {
            self.update_intensity(leftover);
        }
    }

    /// AbstractBuffSimulator.Trim（:26-40）：尾部 End > logEnd 截断。
    fn trim(&mut self, log_end: i64) {
        for item in self.result.items.iter_mut().rev() {
            if item.end > log_end {
                item.end = log_end;
            } else {
                break;
            }
        }
    }
}

// ===== 单元测试（合成事件序列）=====

#[cfg(test)]
mod tests {
    use super::*;
    use gw2ei_parse::BuffStackType as B;

    fn no_heal(_: u32) -> u16 {
        0
    }


    fn run(events: Vec<SimCmd<u32>>, stack_type: B, capacity: i64, log_end: i64) -> SimResult<u32> {
        let mut sim = Simulator::new(stack_type, capacity, &no_heal);
        sim.simulate(&events, 0, log_end);
        sim.result
    }

    /// Queue(Duration)：同源 5 秒 buff 满员（cap 5）后新 apply 淘汰非头中
    /// TotalDuration 最小者（仅头栈走时，非头栈余量不变）。
    #[test]
    fn queue_evicts_min_non_head() {
        let mut ev: Vec<SimCmd<u32>> = Vec::new();
        // t=0..4 连续 5 个 apply(10s)：Queue 无排序，栈按入队序；头栈 t=0 的
        // 持续走时，其余栈余量恒 10_000。
        for i in 0..5u64 {
            ev.push(SimCmd::Add {
                duration: 10_000,
                src: 1,
                start: i as i64,
                stack_id: 100 + i,
                added_active: false,
                overriden_duration: 0,
                overriden_stack_id: 0,
            });
        }
        // 满员后再 apply 12s(t=6)：淘汰非头中 TotalDuration 最小者
        //（= 首个 10_000 余量的栈，即入队序第 2 个,start=1）。
        ev.push(SimCmd::Add {
            duration: 12_000,
            src: 2,
            start: 6,
            stack_id: 200,
            added_active: false,
            overriden_duration: 0,
            overriden_stack_id: 0,
        });
        let r = run(ev, B::Queue, 5, 20_000);
        // 淘汰栈 waste：值=剩余主时长 10_000；时刻=被淘汰栈 Start —— 各栈
        // Start 随每次 Update 前移（C# foreach Shift(diff,0)），淘汰时刻即
        // 当前时刻(6)。
        assert!(r
            .wastes
            .iter()
            .any(|w| w.src == 1 && w.value == 10_000 && w.time == 6));
        assert!(!r.wastes.iter().any(|w| w.src == 2 && w.value == 12_000));
        // 头栈总时长：s0(10s) + 新 12s 栈（t=6 入队,10_000 处转正,余 12s 整
        // —— queue 只走头栈,非头不衰减;末段被 Trim 到 logEnd=20_000）
        let total: i64 = r.items.iter().map(|i| i.end - i.start).sum();
        assert_eq!(total, 20_000);
        // 段 [0..1]（首个事件 t=1 前推进）
        assert_eq!(r.items[0].start, 0);
        assert_eq!(r.items[0].end, 1);
        assert_eq!(r.items[0].groups[0].count, 1);
        // 末段（新栈转正）Trim 到 logEnd
        assert_eq!(r.items.last().expect("last").end, 20_000);
        assert_eq!(r.items.last().expect("last").groups[0].src, 2);
    }

    /// Queue：head 到期 → 队头 extension 转正续命。
    #[test]
    fn duration_head_expiry_promotes_extension() {
        // t=0 apply 5s；t=1000 时同栈 extension +8s（buff change, old=5s>0）
        let ev = vec![
            SimCmd::Add {
                duration: 5_000,
                src: 1,
                start: 0,
                stack_id: 7,
                added_active: false,
                overriden_duration: 0,
                overriden_stack_id: 0,
            },
            SimCmd::Extend {
                extension: 8_000,
                old_value: 5_000,
                src: 2,
                time: 1_000,
                stack_id: 7,
            },
        ];
        let r = run(ev, B::Queue, 5, 20_000);
        // 总存活 = 5s+8s = 13s；两段 attrib：头栈 src=1(前5s) 然后 extension
        // 转正段 src=2 seed=1
        let total: i64 = r.items.iter().map(|i| i.end - i.start).sum();
        assert_eq!(total, 13_000);
        assert_eq!(r.items[0].groups[0].src, 1);
        assert_eq!(r.items[0].groups[0].seed_src, 1);
        let ext_item = r.items.last().expect("item");
        assert_eq!(ext_item.groups[0].src, 2);
        assert_eq!(ext_item.groups[0].seed_src, 1);
        assert!(ext_item.groups[0].is_extension);
    }

    /// Intensity(Override)：同刻多栈计数 + 满员按 TotalDuration 升序淘汰
    /// stacks[0]。
    #[test]
    fn intensity_counts_and_evicts_shortest() {
        let mut ev = vec![SimCmd::Add {
            duration: 10_000,
            src: 1,
            start: 0,
            stack_id: 0,
            added_active: false,
            overriden_duration: 0,
            overriden_stack_id: 0,
        }];
        for t in [100, 200] {
            ev.push(SimCmd::Add {
                duration: 10_000,
                src: 1,
                start: t,
                stack_id: 0,
                added_active: false,
                overriden_duration: 0,
                overriden_stack_id: 0,
            });
        }
        // 满 3（cap 3）；第 4 个 2s 于 t=300 → 淘汰 stacks[0]（同余量时
        // 首栈 t=0,余量 9700）
        ev.push(SimCmd::Add {
            duration: 2_000,
            src: 2,
            start: 300,
            stack_id: 0,
            added_active: false,
            overriden_duration: 0,
            overriden_stack_id: 0,
        });
        let r = run(ev, B::Stacking, 3, 20_000);
        // 段 [200..300]：3 同源栈 → 单组 count=3
        let seg3 = r.items.iter().find(|i| i.start == 200).expect("seg at 200");
        assert_eq!(seg3.groups.len(), 1);
        assert_eq!(seg3.groups[0].count, 3);
        // t=300 淘汰 stacks[0]（waste：值=剩余 9700、时刻=栈 Start=300 ——
        // 全栈 Start 每次 Update 前移）
        assert!(r
            .wastes
            .iter()
            .any(|w| w.src == 1 && w.value == 9_700 && w.time == 300));
        let seg = r.items.iter().find(|i| i.start == 300).expect("seg at 300");
        assert_eq!(seg.groups.iter().map(|g| g.count).sum::<i64>(), 3);
    }

    /// Override logic：Add 二分升序插入（相等插后）。
    #[test]
    fn override_sorts_ascending() {
        let mut sim = Simulator::new(B::Stacking, 10, &no_heal);
        // 直接喂命令，间隔 events 使 update 不干扰插入序
        let ev: Vec<SimCmd<u32>> = vec![
            SimCmd::Add { duration: 9000, src: 1, start: 0, stack_id: 0, added_active: false, overriden_duration: 0, overriden_stack_id: 0 },
            SimCmd::Add { duration: 2000, src: 2, start: 0, stack_id: 0, added_active: false, overriden_duration: 0, overriden_stack_id: 0 },
            SimCmd::Add { duration: 5000, src: 3, start: 0, stack_id: 0, added_active: false, overriden_duration: 0, overriden_stack_id: 0 },
            SimCmd::Add { duration: 5000, src: 4, start: 0, stack_id: 0, added_active: false, overriden_duration: 0, overriden_stack_id: 0 },
        ];
        for e in ev {
            sim.apply_cmd(e);
        }
        let durs: Vec<i64> = sim.stacks.iter().map(|s| s.duration).collect();
        assert_eq!(durs, vec![2000, 5000, 5000, 9000]);
        // 相等(5000)插后：src=4 在 src=3 之后
        let srcs: Vec<u32> = sim.stacks.iter().map(|s| s.src).collect();
        assert_eq!(srcs, vec![2, 3, 4, 1]);
    }

    /// Regen：apply 满员后的 apply 走 overriden 淘汰；Activate 保鲜插头。
    #[test]
    fn regen_override_by_id() {
        let mut sim = Simulator::new(B::Regeneration, 3, &no_heal);
        let apply = |dur: i64, src: u32, t: i64, id: u64| SimCmd::Add {
            duration: dur,
            src,
            start: t,
            stack_id: id,
            added_active: false,
            overriden_duration: 0,
            overriden_stack_id: 0,
        };
        for t in 0..3u64 {
            sim.apply_cmd(apply(10_000, 10 + t as u32, t as i64, 100 + t));
        }
        assert_eq!(sim.stacks.len(), 3);
        // 满员 apply with overridenStackID → 按 id 精确淘汰栈 101
        sim.apply_cmd(SimCmd::Add {
            duration: 12_000,
            src: 9,
            start: 5,
            stack_id: 999,
            added_active: false,
            overriden_duration: 0,
            overriden_stack_id: 101,
        });
        assert_eq!(sim.stacks.len(), 3);
        assert!(!sim.stacks.iter().any(|s| s.src == 11));
        assert!(sim.stacks.iter().any(|s| s.src == 9 && s.duration == 12_000));
    }

    /// RemoveSingle 近似匹配（±15ms）；RemoveAll 全清并记 waste。
    #[test]
    fn remove_single_approx_and_remove_all() {
        // Force cap1：RemoveSingle 近似匹配（余量 9_990，差 10 <15）
        let ev2 = vec![
            SimCmd::Add { duration: 10_000, src: 1, start: 0, stack_id: 0, added_active: false, overriden_duration: 0, overriden_stack_id: 0 },
            SimCmd::Remove { removed_duration: 9_990, time: 10, all: false },
        ];
        let mut sim = Simulator::new(B::Force, 1, &no_heal);
        sim.simulate(&ev2, 0, 50_000);
        assert!(sim.stacks.is_empty());
        assert_eq!(sim.result.wastes.len(), 1);
        assert_eq!(sim.result.wastes[0].value, 10_000 - 10);
        // RemoveAll 清空并逐栈 waste
        let ev3 = vec![
            SimCmd::Add { duration: 10_000, src: 1, start: 0, stack_id: 0, added_active: false, overriden_duration: 0, overriden_stack_id: 0 },
            SimCmd::Remove { removed_duration: 0, time: 100, all: true },
        ];
        let mut sim = Simulator::new(B::Force, 1, &no_heal);
        sim.simulate(&ev3, 0, 50_000);
        assert!(sim.stacks.is_empty());
        assert_eq!(sim.result.wastes[0].value, 10_000 - 100);
        assert_eq!(sim.result.wastes[0].time, 100);
    }

    /// Trim：尾段 End 截断到 logEnd；≤0 段删除。
    #[test]
    fn trim_to_log_end() {
        let ev = vec![SimCmd::Add {
            duration: 100_000,
            src: 1,
            start: 0,
            stack_id: 0,
            added_active: false,
            overriden_duration: 0,
            overriden_stack_id: 0,
        }];
        let r = run(ev, B::Force, 1, 50_000);
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].end, 50_000);
    }
}
