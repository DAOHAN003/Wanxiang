//! 守恒律四池模型 — 严格守恒与漂移吸收
//!
//! 数学基础：Q(t) + S(t) = Q(0) + S(0) = constant
//!   其中 Q = M + E + I（物质/能量/信息），S = 熵池（漂移吸收槽）
//!
//! 三种原子操作：
//! - Create(kind, amount)：池 kind +amount，熵 -amount
//! - Dissipate(kind, amount)：池 kind -amount，熵 +amount
//! - Transfer(from, to, amount)：池间转移，Q+S 不变
//!
//! 每拍调和：drift = observed_Q - ledger_Q → S -= drift → drift = 0

use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// 守恒量种类（熵 S 不在此 enum，因其为吸收槽）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Conserved {
    Matter,
    Energy,
    Information,
}

/// 守恒操作（事件溯源：所有操作均为不可变日志条目）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConservationOp {
    Create { kind: Conserved, amount: f64, tick: u64, op_id: String },
    Dissipate { kind: Conserved, amount: f64, tick: u64, op_id: String },
    Transfer { from: Conserved, to: Conserved, amount: f64, tick: u64, op_id: String },
    /// Snapshot reconciliation is logged so replay reconstructs the same ledger state.
    Reconcile {
        observed_m: f64,
        observed_e: f64,
        observed_i: f64,
        drift: f64,
        tick: u64,
        op_id: String,
    },
}

impl ConservationOp {
    pub fn tick(&self) -> u64 {
        match self {
            ConservationOp::Create { tick, .. }
            | ConservationOp::Dissipate { tick, .. }
            | ConservationOp::Transfer { tick, .. }
            | ConservationOp::Reconcile { tick, .. } => *tick,
        }
    }

    pub fn op_id(&self) -> &str {
        match self {
            ConservationOp::Create { op_id, .. }
            | ConservationOp::Dissipate { op_id, .. }
            | ConservationOp::Transfer { op_id, .. }
            | ConservationOp::Reconcile { op_id, .. } => op_id,
        }
    }
}

/// Input and invariant errors for the resource ledger.
#[derive(Debug, Clone, PartialEq)]
pub enum ConservationError {
    InvalidInitialTotal,
    InvalidAmount,
    InvalidObservation,
    InvalidCurrentState,
    InsufficientBalance { kind: Conserved, available: f64, requested: f64 },
    NonFiniteResult,
}

impl std::fmt::Display for ConservationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInitialTotal => write!(f, "initial total must be finite and non-negative"),
            Self::InvalidAmount => write!(f, "amount must be finite and non-negative"),
            Self::InvalidObservation => write!(f, "observed resource pools must be finite and non-negative"),
            Self::InvalidCurrentState => write!(f, "ledger contains a non-finite or negative resource pool"),
            Self::InsufficientBalance { kind, available, requested } => {
                write!(f, "insufficient {:?} balance: available {}, requested {}", kind, available, requested)
            }
            Self::NonFiniteResult => write!(f, "operation would produce a non-finite ledger state"),
        }
    }
}

impl std::error::Error for ConservationError {}

/// 守恒账本 — 系统单一真理源
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConservationLedger {
    pub matter: f64,
    pub energy: f64,
    pub information: f64,
    pub entropy: f64,
    pub initial_total: f64,
    pub drift: f64,
    pub cumulative_drift: f64,
    pub ops: Vec<ConservationOp>,
    /// 当前 tick（用于 op_id 生成）
    pub current_tick: u64,
    /// tick 内操作序号
    pub tick_op_seq: u64,
}

impl ConservationLedger {
    /// 初始化：所有守恒量都注入 Matter，熵为 0
    pub fn new(initial_total: f64) -> Self {
        assert!(
            initial_total.is_finite() && initial_total >= 0.0,
            "initial_total must be finite and non-negative"
        );
        Self {
            matter: initial_total,
            energy: 0.0,
            information: 0.0,
            entropy: 0.0,
            initial_total,
            drift: 0.0,
            cumulative_drift: 0.0,
            ops: Vec::new(),
            current_tick: 0,
            tick_op_seq: 0,
        }
    }

    fn next_op_id(&mut self, prefix: &str) -> String {
        let id = format!("{}_{}_{}", prefix, self.current_tick, self.tick_op_seq);
        self.tick_op_seq += 1;
        id
    }

    fn pool_get(&self, kind: Conserved) -> f64 {
        match kind {
            Conserved::Matter => self.matter,
            Conserved::Energy => self.energy,
            Conserved::Information => self.information,
        }
    }

    fn pool_add(&mut self, kind: Conserved, delta: f64) {
        match kind {
            Conserved::Matter => self.matter += delta,
            Conserved::Energy => self.energy += delta,
            Conserved::Information => self.information += delta,
        }
    }

    /// Validate an amount before applying any mutation.
    fn validate_amount(&self, amount: f64) -> Result<(), ConservationError> {
        if amount.is_finite() && amount >= 0.0 {
            Ok(())
        } else {
            Err(ConservationError::InvalidAmount)
        }
    }

    fn ensure_finite_state(&self) -> Result<(), ConservationError> {
        let values = [
            self.matter, self.energy, self.information, self.entropy,
            self.initial_total, self.drift, self.cumulative_drift,
        ];
        if values.iter().all(|v| v.is_finite())
            && self.matter >= 0.0
            && self.energy >= 0.0
            && self.information >= 0.0
        {
            Ok(())
        } else {
            Err(ConservationError::InvalidCurrentState)
        }
    }

    /// Create a resource by transferring bookkeeping mass from the signed balance pool.
    pub fn try_create(
        &mut self,
        kind: Conserved,
        amount: f64,
    ) -> Result<&ConservationOp, ConservationError> {
        self.validate_amount(amount)?;
        self.ensure_finite_state()?;
        let pool_after = self.pool_get(kind) + amount;
        let entropy_after = self.entropy - amount;
        if !pool_after.is_finite() || !entropy_after.is_finite()
            || !(self.q() + amount + entropy_after).is_finite()
        {
            return Err(ConservationError::NonFiniteResult);
        }

        let tick = self.current_tick;
        let op_id = self.next_op_id("create");
        self.pool_add(kind, amount);
        self.entropy = entropy_after;
        self.ops.push(ConservationOp::Create { kind, amount, tick, op_id });
        Ok(self.ops.last().expect("operation was just pushed"))
    }

    /// Compatibility wrapper. Invalid input fails closed in debug and release builds.
    pub fn create(&mut self, kind: Conserved, amount: f64) -> &ConservationOp {
        self.try_create(kind, amount).expect("invalid conservation create operation")
    }

    /// Consume resources and move the same bookkeeping amount into the balance pool.
    pub fn try_dissipate(
        &mut self,
        kind: Conserved,
        amount: f64,
    ) -> Result<&ConservationOp, ConservationError> {
        self.validate_amount(amount)?;
        self.ensure_finite_state()?;
        let available = self.pool_get(kind);
        if amount > available {
            return Err(ConservationError::InsufficientBalance { kind, available, requested: amount });
        }
        let pool_after = available - amount;
        let entropy_after = self.entropy + amount;
        if !pool_after.is_finite() || !entropy_after.is_finite()
            || !(self.q() - amount + entropy_after).is_finite()
        {
            return Err(ConservationError::NonFiniteResult);
        }

        let tick = self.current_tick;
        let op_id = self.next_op_id("dissipate");
        self.pool_add(kind, -amount);
        self.entropy = entropy_after;
        self.ops.push(ConservationOp::Dissipate { kind, amount, tick, op_id });
        Ok(self.ops.last().expect("operation was just pushed"))
    }

    /// Compatibility wrapper. Invalid input fails closed in debug and release builds.
    pub fn dissipate(&mut self, kind: Conserved, amount: f64) -> &ConservationOp {
        self.try_dissipate(kind, amount).expect("invalid conservation dissipate operation")
    }

    /// Transfer resources between pools without changing Q + S.
    pub fn try_transfer(
        &mut self,
        from: Conserved,
        to: Conserved,
        amount: f64,
    ) -> Result<&ConservationOp, ConservationError> {
        self.validate_amount(amount)?;
        self.ensure_finite_state()?;
        let available = self.pool_get(from);
        if amount > available {
            return Err(ConservationError::InsufficientBalance { kind: from, available, requested: amount });
        }
        if from != to {
            let from_after = available - amount;
            let to_after = self.pool_get(to) + amount;
            if !from_after.is_finite() || !to_after.is_finite() || !(self.total()).is_finite() {
                return Err(ConservationError::NonFiniteResult);
            }
        }

        let tick = self.current_tick;
        let op_id = self.next_op_id("transfer");
        self.pool_add(from, -amount);
        self.pool_add(to, amount);
        self.ops.push(ConservationOp::Transfer { from, to, amount, tick, op_id });
        Ok(self.ops.last().expect("operation was just pushed"))
    }

    /// Compatibility wrapper. Invalid input fails closed in debug and release builds.
    pub fn transfer(&mut self, from: Conserved, to: Conserved, amount: f64) -> &ConservationOp {
        self.try_transfer(from, to, amount).expect("invalid conservation transfer operation")
    }

    /// 当前守恒总量 Q = M + E + I
    #[inline]
    pub fn q(&self) -> f64 {
        self.matter + self.energy + self.information
    }

    /// 当前 Q + S（应恒等于 initial_total）
    #[inline]
    pub fn total(&self) -> f64 {
        self.q() + self.entropy
    }

    /// 一拍调和：把观测到的实体持有量与账本对齐，差值吸收进熵池
    ///
    /// 数学保证：调和后 Q+S 仍守恒
    ///   Q_new = observed_Q（账本三池对齐到观测值）
    ///   S_new = S_old - drift
    ///   Q_new + S_new = observed_Q + S_old - (observed_Q - ledger_Q) = ledger_Q + S_old = Q_old + S_old ✓
    ///
    /// observed 三池总和由外部世界实体快照给出
    pub fn try_reconcile(
        &mut self,
        observed_m: f64,
        observed_e: f64,
        observed_i: f64,
    ) -> Result<f64, ConservationError> {
        self.ensure_finite_state()?;
        if ![observed_m, observed_e, observed_i].iter().all(|v| v.is_finite() && *v >= 0.0) {
            return Err(ConservationError::InvalidObservation);
        }
        let observed_q = observed_m + observed_e + observed_i;
        let ledger_q = self.q();
        let drift = observed_q - ledger_q;
        let entropy_after = self.entropy - drift;
        let cumulative_after = self.cumulative_drift + drift.abs();
        if !observed_q.is_finite()
            || !drift.is_finite()
            || !entropy_after.is_finite()
            || !cumulative_after.is_finite()
            || !(observed_q + entropy_after).is_finite()
        {
            return Err(ConservationError::NonFiniteResult);
        }

        let tick = self.current_tick;
        let op_id = self.next_op_id("reconcile");
        self.matter = observed_m;
        self.energy = observed_e;
        self.information = observed_i;
        self.entropy = entropy_after;
        self.drift = 0.0;
        self.cumulative_drift = cumulative_after;
        self.ops.push(ConservationOp::Reconcile {
            observed_m, observed_e, observed_i, drift, tick, op_id,
        });
        Ok(drift)
    }

    /// Compatibility wrapper. Invalid input fails closed in debug and release builds.
    pub fn reconcile(&mut self, observed_m: f64, observed_e: f64, observed_i: f64) -> f64 {
        self.try_reconcile(observed_m, observed_e, observed_i)
            .expect("invalid conservation reconciliation")
    }

    /// 推进一拍
    pub fn advance_tick(&mut self) {
        self.current_tick += 1;
        self.tick_op_seq = 0;
    }

    /// 纯函数重建：从初始状态 + 操作序列重建账本（事件溯源核心）
    pub fn reconstruct(initial_total: f64, ops: &[ConservationOp]) -> Self {
        let mut ledger = Self::new(initial_total);
        let mut previous_tick: Option<u64> = None;
        for op in ops {
            let tick = op.tick();
            if previous_tick != Some(tick) {
                ledger.tick_op_seq = 0;
            }
            ledger.current_tick = tick;
            match op {
                ConservationOp::Create { kind, amount, .. } => {
                    ledger.create(*kind, *amount);
                }
                ConservationOp::Dissipate { kind, amount, .. } => {
                    ledger.dissipate(*kind, *amount);
                }
                ConservationOp::Transfer { from, to, amount, .. } => {
                    ledger.transfer(*from, *to, *amount);
                }
                ConservationOp::Reconcile { observed_m, observed_e, observed_i, .. } => {
                    ledger.reconcile(*observed_m, *observed_e, *observed_i);
                }
            }
            previous_tick = Some(tick);
        }
        ledger
    }

    /// 校验：Q + S 是否等于 initial_total（守恒律不变量）
    pub fn check_invariant(&self) -> bool {
        (self.total() - self.initial_total).abs() < 1e-9
    }

    /// 实体级资源映射（外部实体持有的守恒量快照）
    ///
    /// 通常：每实体有 matter_held/energy_held/info_held 字段；
    /// 调用方负责把所有实体的 held 累加，再传给 reconcile。
    pub fn observed_from_entities<'a, I>(entities: I) -> (f64, f64, f64)
    where
        I: IntoIterator<Item = &'a EntityHeld>,
    {
        let (mut m, mut e, mut i) = (0.0, 0.0, 0.0);
        for ent in entities {
            m += ent.matter;
            e += ent.energy;
            i += ent.information;
        }
        (m, e, i)
    }
}

/// 实体持有的守恒量快照（外部结构参考）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntityHeld {
    pub entity_id: String,
    pub matter: f64,
    pub energy: f64,
    pub information: f64,
}

/// 实体持有量聚合（用于 reconcile 前的世界快照采集）
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub entities: HashMap<String, EntityHeld>,
}

impl WorldSnapshot {
    pub fn observed(&self) -> (f64, f64, f64) {
        ConservationLedger::observed_from_entities(self.entities.values())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invariant_holds_after_operations() {
        let mut ledger = ConservationLedger::new(1000.0);
        ledger.create(Conserved::Energy, 100.0);
        ledger.create(Conserved::Information, 50.0);
        ledger.transfer(Conserved::Matter, Conserved::Energy, 30.0);
        ledger.dissipate(Conserved::Information, 20.0);
        assert!(ledger.check_invariant(), "Q+S 必须恒等于 initial_total");
        assert!(ledger.cumulative_drift == 0.0, "未调和时累积漂移应记 0");
    }

    #[test]
    fn reconcile_absorbs_drift_into_entropy() {
        let mut ledger = ConservationLedger::new(1000.0);
        // 实体观测：M=900, E=80, I=40 → Q_obs=1020 > Q_ledger=1000 → drift=+20
        let drift = ledger.reconcile(900.0, 80.0, 40.0);
        assert!((drift - 20.0).abs() < 1e-9, "drift 应为 +20");
        assert!(ledger.check_invariant(), "吸收后 Q+S 仍守恒");
        assert!((ledger.entropy - (-20.0)).abs() < 1e-9, "熵池应 -20");
    }

    #[test]
    fn reconstruct_is_pure_function() {
        let mut ledger = ConservationLedger::new(500.0);
        ledger.create(Conserved::Energy, 100.0);
        ledger.transfer(Conserved::Matter, Conserved::Information, 50.0);
        ledger.dissipate(Conserved::Energy, 30.0);
        let ops = ledger.ops.clone();

        let rebuilt = ConservationLedger::reconstruct(500.0, &ops);
        assert!((rebuilt.matter - ledger.matter).abs() < 1e-9);
        assert!((rebuilt.energy - ledger.energy).abs() < 1e-9);
        assert!((rebuilt.information - ledger.information).abs() < 1e-9);
        assert!((rebuilt.entropy - ledger.entropy).abs() < 1e-9);
        assert!(rebuilt.check_invariant());
    }

    #[test]
    fn reconcile_is_included_in_event_replay() {
        let mut ledger = ConservationLedger::new(1000.0);
        ledger.create(Conserved::Energy, 80.0);
        ledger.reconcile(850.0, 100.0, 40.0);
        ledger.transfer(Conserved::Matter, Conserved::Information, 20.0);

        let rebuilt = ConservationLedger::reconstruct(1000.0, &ledger.ops.clone());
        assert!((rebuilt.matter - ledger.matter).abs() < 1e-9);
        assert!((rebuilt.energy - ledger.energy).abs() < 1e-9);
        assert!((rebuilt.information - ledger.information).abs() < 1e-9);
        assert!((rebuilt.entropy - ledger.entropy).abs() < 1e-9);
        assert!((rebuilt.cumulative_drift - ledger.cumulative_drift).abs() < 1e-9);
        assert_eq!(rebuilt.ops.len(), ledger.ops.len());
        assert!(rebuilt.check_invariant());
    }

    #[test]
    fn invalid_amounts_and_overdrafts_are_rejected_without_mutation() {
        let mut ledger = ConservationLedger::new(100.0);
        let before = ledger.total();
        assert!(matches!(
            ledger.try_create(Conserved::Energy, f64::NAN),
            Err(ConservationError::InvalidAmount)
        ));
        assert!(matches!(
            ledger.try_dissipate(Conserved::Matter, 101.0),
            Err(ConservationError::InsufficientBalance { .. })
        ));
        assert!(matches!(
            ledger.try_transfer(Conserved::Energy, Conserved::Matter, -1.0),
            Err(ConservationError::InvalidAmount)
        ));
        assert_eq!(ledger.total(), before);
        assert!(ledger.ops.is_empty());
    }

    #[test]
    fn invalid_reconcile_is_rejected_without_mutation() {
        let mut ledger = ConservationLedger::new(100.0);
        assert!(matches!(
            ledger.try_reconcile(10.0, -1.0, 5.0),
            Err(ConservationError::InvalidObservation)
        ));
        assert_eq!(ledger.matter, 100.0);
        assert!(ledger.ops.is_empty());
    }

    #[test]
    fn deterministic_op_ids_within_tick() {
        let mut ledger = ConservationLedger::new(100.0);
        ledger.create(Conserved::Matter, 1.0);
        ledger.create(Conserved::Matter, 1.0);
        ledger.advance_tick();
        ledger.create(Conserved::Matter, 1.0);
        assert_eq!(ledger.ops[0].op_id(), "create_0_0");
        assert_eq!(ledger.ops[1].op_id(), "create_0_1");
        assert_eq!(ledger.ops[2].op_id(), "create_1_0");
    }
}
