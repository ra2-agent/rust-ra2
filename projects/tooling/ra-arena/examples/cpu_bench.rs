//! 串行固定赛程的分配计数基准；计数器会增加开销，耗时应另用普通 CLI 验证。

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};

use ra_arena::{ArenaConfig, BotRegistry, Scenario, run_tournament};

struct CountingAllocator;
static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static REALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);

fn count(bytes: usize, reallocation: bool) {
    if COUNTING.load(Ordering::Relaxed) {
        if reallocation {
            REALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        else {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        REQUESTED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size(), false);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size(), false);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size, true);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn main() {
    let rounds = std::env::args().nth(1).map(|value| value.parse().expect("rounds 必须为正整数")).unwrap_or(10);
    let registry = BotRegistry::builtins();
    let config = ArenaConfig {
        npcs: vec!["idle-v1".into(), "easy-v1".into(), "normal-v1".into(), "hard-v1".into()],
        scenarios: vec![Scenario::Duel, Scenario::Skirmish],
        rounds,
        jobs: 1,
        summary_only: true,
        ..ArenaConfig::default()
    };
    let started = Instant::now();
    COUNTING.store(true, Ordering::Relaxed);
    let report = run_tournament(&registry, config).expect("固定赛程应成功");
    COUNTING.store(false, Ordering::Relaxed);
    let wall_seconds = started.elapsed().as_secs_f64();
    println!(
        "{}",
        serde_json::json!({
            "rounds": rounds,
            "wall_seconds_with_allocation_counters": wall_seconds,
            "allocations": ALLOCATIONS.load(Ordering::Relaxed),
            "reallocations": REALLOCATIONS.load(Ordering::Relaxed),
            "requested_bytes_including_reallocations": REQUESTED_BYTES.load(Ordering::Relaxed),
            "statistics": report.statistics(),
            "leaderboard": report.leaderboard,
            "by_scenario": report.by_scenario,
        })
    );
}
