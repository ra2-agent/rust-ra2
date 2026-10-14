//! 有界分项耗时汇总、加权直方图和导出；不保存每 tick 的计时记录。

use std::{collections::BTreeMap, fmt::Write};

use serde::{Deserialize, Serialize};

/// 跨局合并的纳秒统计；P95 从 self 时间的 log2 直方图取上界。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingStats {
    /// 实际计时调用数，抽样模式下不外推。
    pub calls: u64,
    /// 包含已插桩子调用的累计时间。
    pub inclusive_ns: u64,
    /// 扣除已插桩子调用的累计时间。
    pub self_ns: u64,
    /// 单次 inclusive 最大时间。
    pub max_inclusive_ns: u64,
    /// 单次 self 最大时间。
    pub max_self_ns: u64,
    /// 64 个 self 纳秒 log2 桶的调用数；合并后按调用数计算分位数。
    pub self_histogram: Vec<u64>,
}

impl TimingStats {
    /// 合并计数与直方图，不平均各局的均值或分位数。
    pub fn merge(&mut self, other: &Self) {
        self.calls = self.calls.saturating_add(other.calls);
        self.inclusive_ns = self.inclusive_ns.saturating_add(other.inclusive_ns);
        self.self_ns = self.self_ns.saturating_add(other.self_ns);
        self.max_inclusive_ns = self.max_inclusive_ns.max(other.max_inclusive_ns);
        self.max_self_ns = self.max_self_ns.max(other.max_self_ns);
        self.self_histogram.resize(64, 0);
        for (target, count) in self.self_histogram.iter_mut().zip(&other.self_histogram) {
            *target = target.saturating_add(*count);
        }
    }

    /// 平均 self 时间，单位微秒；没有调用时为 0。
    pub fn mean_self_us(&self) -> f64 {
        if self.calls == 0 { 0.0 } else { self.self_ns as f64 / self.calls as f64 / 1000.0 }
    }

    /// 近似 P95 self 纳秒上界；没有调用时为 `None`，桶精度最多约二倍。
    pub fn p95_self_upper_ns(&self) -> Option<u64> {
        if self.calls == 0 {
            return None;
        }
        let rank = (u128::from(self.calls) * 95).div_ceil(100);
        let mut seen = 0_u128;
        for (bucket, &count) in self.self_histogram.iter().take(64).enumerate() {
            seen += u128::from(count);
            if seen >= rank {
                return Some(if bucket == 63 { u64::MAX } else { 1_u64 << bucket });
            }
        }
        Some(self.max_self_ns)
    }
}

/// 一组比赛的组件计时；采样统计与整个对局墙钟时间分别保存。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileGroup {
    /// 完成的比赛数。
    pub games: u64,
    /// 实际观察的仿真 tick 数。
    pub observed_ticks: u64,
    /// 测量的 tick 数。
    pub sampled_ticks: u64,
    /// 各对局捕获的完整墙钟时间之和；并发时可超过比赛场总耗时。
    pub match_wall_ns: u64,
    /// 分项耗时；只存实际调用过的分类。
    pub timings: BTreeMap<String, TimingStats>,
}

/// 可保存与导出的组件耗时报告，与游戏胜负和经济遥测独立。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArenaProfile {
    /// 计时格式版本。
    pub version: String,
    /// 时钟口径；为 monotonic_wall_time，不能当作进程 CPU 时间。
    pub clock: String,
    /// 每多少 tick 测量一次；装载与结算始终测量。
    pub sample_every: u64,
    /// 整次循环赛的墙钟时间，不含最终 JSON / Markdown / CSV 输出。
    pub tournament_wall_ns: u64,
    /// 全场累计统计。
    pub overall: ProfileGroup,
    /// 按场景累计统计。
    pub by_scenario: BTreeMap<String, ProfileGroup>,
    /// 各 NPC 决策函数的计时，跨左右席位合并。
    pub npc_decisions: BTreeMap<String, TimingStats>,
}

impl ArenaProfile {
    pub(crate) fn new(sample_every: u64) -> Self {
        Self {
            version: "ra-arena-timing-v1".into(),
            clock: "monotonic_wall_time".into(),
            sample_every,
            tournament_wall_ns: 0,
            overall: ProfileGroup::default(),
            by_scenario: BTreeMap::new(),
            npc_decisions: BTreeMap::new(),
        }
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn record(&mut self, scene: &str, seats: &[String; 2], data: ra_engine::profiling::ProfileData) {
        use ra_engine::profiling::Stage;
        let mut rows = BTreeMap::<String, TimingStats>::new();
        for (stage, stats) in Stage::ALL.iter().zip(data.stages) {
            if stats.calls == 0 {
                continue;
            }
            let stats = TimingStats {
                calls: stats.calls,
                inclusive_ns: stats.inclusive_ns,
                self_ns: stats.self_ns,
                max_inclusive_ns: stats.max_inclusive_ns,
                max_self_ns: stats.max_self_ns,
                self_histogram: stats.histogram.to_vec(),
            };
            let key = match stage {
                Stage::NpcLeft | Stage::NpcRight => {
                    let seat = usize::from(*stage == Stage::NpcRight);
                    self.npc_decisions.entry(seats[seat].clone()).or_default().merge(&stats);
                    "npc_decision"
                }
                _ => stage.key(),
            };
            rows.entry(key.to_string()).or_default().merge(&stats);
        }
        for group in [&mut self.overall, self.by_scenario.entry(scene.to_string()).or_default()] {
            group.games += 1;
            group.observed_ticks += data.observed_ticks;
            group.sampled_ticks += data.sampled_ticks;
            group.match_wall_ns = group.match_wall_ns.saturating_add(data.elapsed_ns);
            for (key, stats) in &rows {
                group.timings.entry(key.clone()).or_default().merge(stats);
            }
        }
    }

    /// 生成人类可读的 self 耗时排名、场景分项与 NPC 决策统计。
    pub fn markdown(&self) -> String {
        let mut result = format!(
            "\n## 分组件耗时\n\n格式 `{}`；时钟 `{}`；每 {} tick 采样一次，共 {} / {} tick，{} 局。\n\n\
循环赛墙钟 {:.3} 秒；对局捕获墙钟之和 {:.3} 秒。下表占比使用已测组件 self 时间之和，\
inclusive 含已插桩子调用，不能再次相加；P95 为 log2 桶上界。抽样累计时间没有外推。\n\n",
            self.version,
            self.clock,
            self.sample_every,
            self.overall.sampled_ticks,
            self.overall.observed_ticks,
            self.overall.games,
            self.tournament_wall_ns as f64 / 1e9,
            self.overall.match_wall_ns as f64 / 1e9,
        );
        table(&mut result, &self.overall.timings);
        for (scene, group) in &self.by_scenario {
            writeln!(&mut result, "\n### {scene} 分组件耗时\n").unwrap();
            table(&mut result, &group.timings);
        }
        result.push_str("\n### NPC 决策耗时\n\n");
        table(&mut result, &self.npc_decisions);
        result.push_str("\n计时插桩有额外开销，短函数尤其容易受影响；定位热点优先使用串行 release 与多次固定种子运行。并发累计时间含线程被调度暂停的等待，可能大于循环赛墙钟时间；总 CPU 开销使用进程 CPU 基准另测。\n");
        result
    }

    /// CSV 长表，包含全场、场景及 NPC 分组，可直接导入统计工具。
    pub fn csv(&self) -> String {
        let mut result =
            "scope,group,component,calls,self_ms,inclusive_ms,self_share_pct,mean_self_us,p95_self_upper_us,max_self_us,max_inclusive_us\n"
                .to_string();
        csv_rows(&mut result, "overall", "all", &self.overall.timings);
        for (scene, group) in &self.by_scenario {
            csv_rows(&mut result, "scenario", scene, &group.timings);
        }
        csv_rows(&mut result, "npc", "decision", &self.npc_decisions);
        result
    }
}

fn table(result: &mut String, rows: &BTreeMap<String, TimingStats>) {
    result.push_str("| 组件 | 自身 ms | 自身占比 | 调用数 | 平均自身 µs | P95自身≤µs | 最大自身 µs | inclusive ms |\n|---|---:|---:|---:|---:|---:|---:|---:|\n");
    let total = rows.values().map(|row| u128::from(row.self_ns)).sum::<u128>() as f64;
    let mut ordered: Vec<_> = rows.iter().collect();
    ordered.sort_by(|a, b| b.1.self_ns.cmp(&a.1.self_ns).then_with(|| a.0.cmp(b.0)));
    for (key, row) in ordered {
        writeln!(
            result,
            "| {key} | {:.3} | {:.2}% | {} | {:.3} | {} | {:.3} | {:.3} |",
            row.self_ns as f64 / 1e6,
            share(row.self_ns, total),
            row.calls,
            row.mean_self_us(),
            row.p95_self_upper_ns().map(|ns| format!("{:.3}", ns as f64 / 1000.0)).unwrap_or_else(|| "—".into()),
            row.max_self_ns as f64 / 1000.0,
            row.inclusive_ns as f64 / 1e6
        )
        .unwrap();
    }
}

fn csv_rows(result: &mut String, scope: &str, group: &str, rows: &BTreeMap<String, TimingStats>) {
    let total = rows.values().map(|row| u128::from(row.self_ns)).sum::<u128>() as f64;
    for (key, row) in rows {
        let p95 = row.p95_self_upper_ns().map(|ns| format!("{:.6}", ns as f64 / 1000.0)).unwrap_or_default();
        writeln!(
            result,
            "{scope},{group},{key},{},{:.6},{:.6},{:.6},{:.6},{p95},{:.6},{:.6}",
            row.calls,
            row.self_ns as f64 / 1e6,
            row.inclusive_ns as f64 / 1e6,
            share(row.self_ns, total),
            row.mean_self_us(),
            row.max_self_ns as f64 / 1000.0,
            row.max_inclusive_ns as f64 / 1000.0
        )
        .unwrap();
    }
}

fn share(ns: u64, total: f64) -> f64 {
    if total == 0.0 { 0.0 } else { ns as f64 / total * 100.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_weights_by_calls_and_histograms_instead_of_averaging_percentiles() {
        let mut many = TimingStats {
            calls: 99,
            self_ns: 99_000,
            inclusive_ns: 99_000,
            max_self_ns: 1000,
            self_histogram: vec![0; 64],
            ..TimingStats::default()
        };
        many.self_histogram[10] = 99;
        let mut rare = TimingStats {
            calls: 1,
            self_ns: 1_000_000,
            inclusive_ns: 1_000_000,
            max_self_ns: 1_000_000,
            self_histogram: vec![0; 64],
            ..TimingStats::default()
        };
        rare.self_histogram[20] = 1;
        many.merge(&rare);
        assert_eq!(many.calls, 100);
        assert_eq!(many.p95_self_upper_ns(), Some(1024));
        assert_eq!(many.mean_self_us(), 10.99);
        assert_eq!(many.max_self_ns, 1_000_000);
        assert_eq!(TimingStats::default().p95_self_upper_ns(), None);
    }
}
