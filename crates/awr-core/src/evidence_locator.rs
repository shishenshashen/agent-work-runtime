//! FEAT-PEL-001 · Portable evidence locator — **contract skeleton** (gate 1).
//!
//! 状态：**未编译**（本文件落盘后由 gate 1 `cargo check -p awr-core` 判定；结果另报）。
//! 本文件只放设计 v2.2 的类型与函数签名，函数体为 `unimplemented!()`；**无实现、无测试**。
//! 设计依据：`reports/feature-portable-evidence-locator-design-v2.2.md`（九条阻断落点）。
//! 绝不改写 `Evidence.locator` 原文；本模块只产出新的归一结构。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Id;

/// 归一后的 locator 类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocatorKind {
    /// 位于 project root 之内，跨设备稳定（只有相对结构参与判据）。
    ProjectRelative,
    /// 绝对路径且落在 root 之外：不可移植，显式标记而不是静默当作普通记录。
    ExternalAbsolute,
    /// 其它形式（非 `file` scheme、纯标识符、不可解析）。
    Opaque,
}

/// 不可移植的原因，与"缺失/漂移"严格区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnportableReason {
    /// 词法折叠后（normalize）或 canonicalize 后（resolve）逃出 root。
    PathEscape,
    /// ExternalAbsolute / Opaque / 不可表示的输入。
    NotPortable,
}

/// 归一结果。`original` 逐字节保留输入，永不改写。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceLocatorV1 {
    pub kind: LocatorKind,
    /// `'/'` 分隔、无 `.` / `..` 段（仅 ProjectRelative 时非空）。
    pub relpath: Option<String>,
    pub line: Option<u32>,
    pub span: Option<u32>,
    pub content_sha256: Option<String>,
    /// 该证据绑定的节指纹（来自 `SourceRef.section_fingerprint`）。
    pub section_expected: Option<String>,
    /// 写证据的设备标签：仅诊断，**不作判据**。
    pub origin_hint: Option<String>,
    pub original: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IoOp {
    Canonicalize,
    Hash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolveOutcome {
    Current { sha256: String },
    /// 文件在、但该证据没有绑定内容指纹：**不得**当作 Current。
    Unbound,
    Missing,
    ContentDrift { expected: String, actual: String },
    SectionDrift {
        expected: Option<String>,
        actual: Option<String>,
    },
    Unportable(UnportableReason),
    /// 环境错误 ≠ Missing：不许静默折叠。
    IoFailed { op: IoOp },
}

/// 由 `resolve` 自己累加的 IO 计数（替代已删除的 `ReadCounter`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IoStats {
    pub canonicalize_calls: u32,
    pub hash_reads: u32,
}

/// 节指纹来源：#336 未落 / yaml 台账 ⇒ `Unavailable`（不是错误）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectionCheck {
    Unavailable,
    Available { actual: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectionStatus {
    NotEvaluated,
    NotBound,
    Compared { equal: bool },
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveReport {
    pub outcome: ResolveOutcome,
    pub section: SectionStatus,
    pub io: IoStats,
}

/// 注入面：PR-1 只依赖这一个 trait；生产实现仅用于真实文件系统冒烟。
pub trait LocatorIo {
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf>;
    fn sha256_file(&self, path: &Path) -> std::io::Result<String>;
}

/// PR-2 才实现；此处只定义契约。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableGroup {
    pub relpath: String,
    pub line: Option<u32>,
    pub span: Option<u32>,
    pub evidence_ids: Vec<Id>,
    pub outcome: ResolveOutcome,
    pub section_check_unavailable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnportableEntry {
    pub original: String,
    pub reason: UnportableReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableReport {
    pub groups: Vec<PortableGroup>,
    pub unportable: Vec<UnportableEntry>,
}

/// 纯函数、零 IO、宿主无关：rules per design v2.2 §0 C1/C2。
pub fn normalize(
    _locator: &str,
    _project_root: &Path,
    _content_sha256: Option<String>,
    _section_expected: Option<String>,
    _origin_hint: Option<String>,
) -> Result<EvidenceLocatorV1, UnportableReason> {
    unimplemented!("gate-1 contract skeleton: normalize is not implemented")
}

pub fn resolve(
    _loc: &EvidenceLocatorV1,
    _local_root: &Path,
    _io: &dyn LocatorIo,
    _section: SectionCheck,
) -> ResolveReport {
    unimplemented!("gate-1 contract skeleton: resolve is not implemented")
}
