use std::{
    env, fs,
    io::{BufWriter, Write},
    path::PathBuf,
    process::{Command, ExitCode},
    time::Instant,
};

use ra_arena::{ArenaConfig, ArenaResult, BotRegistry, NpcMetrics, PROTOCOL, Rating, Report, Scenario, replay_match, run_tournament};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ra-arena: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> ArenaResult<()> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "ra-arena — NPC 镜像循环赛与 Elo 评测\n\
\n用法：\n\
  cargo run -p ra-arena -- list\n\
  cargo run -p ra-arena -- run [选项]\n\
  cargo run -p ra-arena -- leaderboard <report.json>\n\
  cargo run -p ra-arena -- replay <report.json>\n\
  cargo run -p ra-arena -- profile <report.json>\n\
\n选项：\n\
  --config <path>       读取 ArenaConfig JSON；其他选项覆盖配置\n\
  --bots <id,id,...>    参赛 NPC，默认 idle/easy/normal/hard-v1\n\
  --scenarios <id,...>  默认 duel-v1,skirmish-v1\n\
  --rounds <n>         每对 NPC 每场景镜像双局次数，默认 4\n\
  --seed <n>           固定赛程种子，默认 42\n\
  --max-ticks <n>      每局最多 tick 数，默认 3000\n\
  --max-commands <n>   每 NPC 每 tick 命令上限，默认 64\n\
  --initial <n>        初始 Elo，默认 1500\n\
  --k <n>              每个镜像双局的 K，默认 32\n\
  --trace              保存命令轨迹，允许 replay\n\
  --no-trace           关闭配置中的命令轨迹\n\
  --jobs <n|auto>      并发镜像双局数，默认 1；越高 CPU 和内存占用越高\n\
  --summary            只保留排名和统计，内存/报告大小不随轮次增长\n\
  --pretty             输出缩进 JSON，默认紧凑 JSON\n\
  --profile            分组件计时，须用 --features profiling 构建\n\
  --profile-every <n>  每 n tick 采样一次，默认 1；累计时间不外推\n\
  --out <path>         默认 arena-results/report.json，另导出同名 Markdown\n\
\n比赛使用当前 Rust 引擎与合成规则、全图信息协议。每次 run 独立从初始分计分。"
        );
        return Ok(());
    }
    let action = if args.first().is_some_and(|arg| !arg.starts_with('-')) { args.remove(0) } else { "run".into() };
    let registry = BotRegistry::builtins();
    match action.as_str() {
        "list" => {
            if !args.is_empty() {
                return Err("list 不接受参数".into());
            }
            println!("NPC: {}\n场景: duel-v1, skirmish-v1, economy-v1\n协议: {PROTOCOL}", registry.ids().join(", "));
        }
        "leaderboard" | "replay" | "profile" => {
            if args.len() != 1 {
                return Err(format!("{action} 需要一个 report.json 路径"));
            }
            let report: Report = serde_json::from_slice(&fs::read(&args[0]).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            if report.protocol != PROTOCOL {
                return Err("报告协议不匹配".into());
            }
            if action == "profile" {
                println!("{}", report.profiling.as_ref().ok_or("此报告未开启 --profile，没有组件计时")?.markdown());
            }
            else if action == "leaderboard" {
                println!("{}", markdown(&report));
            }
            else {
                if report.config.summary_only || !report.config.trace || report.matches.is_empty() {
                    return Err("此报告没有可回放的比赛轨迹；请去掉 --summary 并加 --trace 重新运行".into());
                }
                for (i, record) in report.matches.iter().enumerate() {
                    replay_match(record).map_err(|e| format!("第 {} 局: {e}", i + 1))?;
                }
                println!("回放验证通过：{} 局，命令、终局哈希及胜负一致。", report.matches.len());
            }
        }
        "run" => {
            let (config, out, pretty) = parse_config(&args)?;
            config.validate(&registry)?;
            let games = config.match_count()?;
            eprintln!(
                "正在运行 {games} 局，{} 轮，seed={}，每局上限 {} tick，并发 {}，{}报告…",
                config.rounds,
                config.seed,
                config.max_ticks,
                config.jobs,
                if config.summary_only { "精简" } else { "完整" }
            );
            let started = Instant::now();
            let mut report = run_tournament(&registry, config)?;
            let elapsed = started.elapsed().as_secs_f64();
            let stats = report.statistics();
            eprintln!("完成：{elapsed:.3} 秒，{:.1} 局/秒，{:.0} tick/秒。", stats.games as f64 / elapsed, stats.ticks as f64 / elapsed);
            report.source_revision = git(&["rev-parse", "HEAD"]);
            report.source_dirty = git(&["status", "--porcelain"]).map(|status| !status.is_empty());
            if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let output = markdown(&report);
            // 直接写缓冲文件，不额外分配一份可能很大的 JSON 字节数组。
            let mut writer = BufWriter::new(fs::File::create(&out).map_err(|e| e.to_string())?);
            if pretty { serde_json::to_writer_pretty(&mut writer, &report) } else { serde_json::to_writer(&mut writer, &report) }
                .map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
            let md_out = out.with_extension("md");
            fs::write(&md_out, &output).map_err(|e| e.to_string())?;
            if let Some(profile) = &report.profiling {
                let csv_out = out.with_extension("profile.csv");
                fs::write(&csv_out, profile.csv()).map_err(|e| e.to_string())?;
                eprintln!("组件计时 CSV：{}", csv_out.display());
            }
            let csv_out = out.with_extension("csv");
            fs::write(&csv_out, metrics_csv(&report)).map_err(|e| e.to_string())?;
            println!("{output}\n报告：{}\n摘要：{}\n指标：{}", out.display(), md_out.display(), csv_out.display());
        }
        _ => return Err(format!("未知操作 {action}；使用 --help 查看用法")),
    }
    Ok(())
}

fn parse_config(args: &[String]) -> ArenaResult<(ArenaConfig, PathBuf, bool)> {
    let mut config = ArenaConfig::default();
    let mut out = PathBuf::from("arena-results/report.json");
    let mut pretty = false;
    // 先加载配置，使 CLI 覆盖不受参数排列位置影响。
    let configs: Vec<_> = args.iter().enumerate().filter(|(_, arg)| arg.as_str() == "--config").collect();
    if configs.len() > 1 {
        return Err("--config 只能出现一次".into());
    }
    if let Some((index, _)) = configs.first() {
        let path = args.get(index + 1).ok_or("--config 缺少路径")?;
        config = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| format!("配置错误: {e}"))?;
    }
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        if matches!(flag, "--trace" | "--no-trace" | "--summary" | "--pretty" | "--profile") {
            match flag {
                "--trace" => config.trace = true,
                "--no-trace" => config.trace = false,
                "--summary" => config.summary_only = true,
                "--pretty" => pretty = true,
                "--profile" => config.profile = true,
                _ => unreachable!(),
            }
            i += 1;
            continue;
        }
        let value = args.get(i + 1).ok_or_else(|| format!("{flag} 缺少值"))?;
        let invalid = || format!("{flag} 的值无效: {value}");
        match flag {
            "--config" => {}
            "--bots" => config.npcs = value.split(',').map(|v| v.trim().to_string()).collect(),
            "--scenarios" => config.scenarios = value.split(',').map(|v| Scenario::parse(v.trim())).collect::<ArenaResult<_>>()?,
            "--rounds" => config.rounds = value.parse().map_err(|_| invalid())?,
            "--seed" => config.seed = value.parse().map_err(|_| invalid())?,
            "--max-ticks" => config.max_ticks = value.parse().map_err(|_| invalid())?,
            "--max-commands" => config.max_commands_per_tick = value.parse().map_err(|_| invalid())?,
            "--profile-every" => config.profile_sample_every = value.parse().map_err(|_| invalid())?,
            "--jobs" => {
                config.jobs = if value == "auto" {
                    std::thread::available_parallelism().map_err(|e| e.to_string())?.get()
                }
                else {
                    value.parse().map_err(|_| invalid())?
                };
            }
            "--initial" => config.elo.initial = value.parse().map_err(|_| invalid())?,
            "--k" => config.elo.k = value.parse().map_err(|_| invalid())?,
            "--out" => {
                out = PathBuf::from(value);
                if out.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    return Err("--out 必须为 .json 文件".into());
                }
            }
            _ => return Err(format!("未知选项: {flag}")),
        }
        i += 2;
    }
    Ok((config, out, pretty))
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn rating_table(rows: &[Rating], provisional_games: u64) -> String {
    let mut result = "| NPC | Elo | 局数 | 胜 | 和 | 负 | 得分率 | 状态 |\n|---|---:|---:|---:|---:|---:|---:|---|\n".to_string();
    for row in rows {
        result.push_str(&format!(
            "| {} | {:.1} | {} | {} | {} | {} | {:.1}% | {} |\n",
            row.npc,
            row.elo,
            row.games(),
            row.wins,
            row.draws,
            row.losses,
            row.score_rate() * 100.0,
            if row.games() < provisional_games { "暂定" } else { "已达最低样本数" }
        ));
    }
    result
}

fn markdown(report: &Report) -> String {
    let stats = report.statistics();
    let mut result = format!(
        "# NPC Elo 评测\n\n协议 `{}`；合成规则 `{}`；引擎 {}；seed={}；共 {} 局。\n\n\
Elo 初始 {}，K={}（每个镜像双局），分差尺度 {}；超时平局 {} 局；拒绝命令 {}；截断命令 {}。\n\n\
## 综合排名\n\n{}",
        report.protocol,
        report.rules_hash,
        report.engine_version,
        report.config.seed,
        stats.games,
        report.config.elo.initial,
        report.config.elo.k,
        report.config.elo.scale,
        stats.timeouts,
        stats.rejected,
        stats.dropped,
        rating_table(&report.leaderboard, report.config.elo.provisional_games)
    );
    if report.config.summary_only {
        result.push_str("\n精简报告仅保留累计统计；未保存逐局记录与回放轨迹。\n");
    }
    result.push_str(&format!("\n## 实测指标\n\n{}", metrics_table(&report.statistics().metrics, &report.leaderboard)));
    for (scene, rows) in &report.by_scenario {
        result.push_str(&format!("\n## {scene}\n\n{}", rating_table(rows, report.config.elo.provisional_games)));
        if let Some(metrics) = stats.metrics_by_scenario.get(scene) {
            result.push_str(&format!("\n{}", metrics_table(metrics, rows)));
        }
    }
    result.push_str("\n## 交手得分率\n\n行 NPC 对列 NPC 的胜局加半数和局 / 交手局数；括号内为局数。\n\n| NPC |");
    for id in &report.config.npcs {
        result.push_str(&format!(" {id} |"));
    }
    result.push_str("\n|---|");
    for _ in &report.config.npcs {
        result.push_str("---:|");
    }
    result.push('\n');
    for a in &report.config.npcs {
        result.push_str(&format!("| {a} |"));
        for b in &report.config.npcs {
            match stats.head_to_head.get(a).and_then(|row| row.get(b)).filter(|m| m.games > 0) {
                Some(m) => result.push_str(&format!(" {:.1}% ({}) |", m.points / m.games as f64 * 100.0, m.games)),
                None => result.push_str(" — |"),
            }
        }
        result.push('\n');
    }
    result.push_str("\n这些分数只适用于本配置下的 Rust 引擎和合成场景。全图信息协议未模拟战争迷雾。达到最低样本数也不等于统计收敛。\n");
    result.push_str("\n资源速度 = 实际卸矿到账 / 总仿真时长，单位资金/游戏分钟（15 tick/秒）；前60秒速率按各局实际窗口时长加权。平均对局分为胜1、和0.5、负0；平均游戏积分沿用引擎临时公式 max(0, 支出/100 + 击杀×10 − 损失×5)。首次收入仅对有收入局求均值；未采矿为 —，旧报告无遥测为未知。经济场景不与无矿场景混合解读。\n");
    if let Some(profile) = &report.profiling {
        result.push_str(&profile.markdown());
    }
    result
}

fn metrics_table(metrics: &std::collections::BTreeMap<String, NpcMetrics>, ratings: &[Rating]) -> String {
    if metrics.is_empty() {
        return "此报告无遥测指标（未知）。\n".into();
    }
    let mut result = "| NPC | 平均对局分 | 平均游戏积分 | 采矿/游戏分钟 | 前60秒采矿/分钟 | 首次收入秒 | 有收入局/总局 | 平均支出 | 拒绝率 |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|\n".to_string();
    for row in ratings {
        if let Some(m) = metrics.get(&row.npc) {
            let first = first_income_seconds(m).map(|v| format!("{v:.2}")).unwrap_or_else(|| "—".into());
            result.push_str(&format!(
                "| {} | {:.3} | {:.2} | {:.2} | {:.2} | {} | {}/{} | {:.2} | {:.2}% |\n",
                row.npc,
                row.score_rate(),
                m.average_battle_score(),
                m.income_per_minute(),
                m.opening_income_per_minute(),
                first,
                m.income_games,
                m.games,
                average(m.spent as f64, m.games),
                average(m.rejected as f64, m.submitted) * 100.0
            ));
        }
    }
    result
}

fn average(total: f64, count: u64) -> f64 {
    if count == 0 { 0.0 } else { total / count as f64 }
}

fn first_income_seconds(m: &NpcMetrics) -> Option<f64> {
    (m.income_games > 0).then(|| average(m.first_income_ticks as f64, m.income_games) / f64::from(ra_engine::DEFAULT_TICK_HZ))
}

fn metrics_csv(report: &Report) -> String {
    let stats = report.statistics();
    let mut csv = "scenario,npc,games,elo,avg_match_points,win_rate,avg_battle_score,harvested_credits,income_per_game_minute,opening_income_per_game_minute,avg_first_income_seconds,income_games,avg_spent,avg_final_funds,avg_kills,avg_losses,avg_built,low_power_pct,rejected_pct,submitted,dropped\n".to_string();
    for (scene, ratings, metrics) in std::iter::once(("overall", &report.leaderboard, &stats.metrics)).chain(
        report
            .by_scenario
            .iter()
            .filter_map(|(scene, ratings)| stats.metrics_by_scenario.get(scene).map(|metrics| (scene.as_str(), ratings, metrics))),
    ) {
        for row in ratings {
            let Some(m) = metrics.get(&row.npc)
            else {
                continue;
            };
            let first = first_income_seconds(m).map(|v| format!("{v:.6}")).unwrap_or_default();
            csv.push_str(&format!(
                "{scene},{},{},{:.6},{:.6},{:.6},{:.6},{},{:.6},{:.6},{first},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{}\n",
                row.npc,
                m.games,
                row.elo,
                row.score_rate(),
                average(row.wins as f64, row.games()),
                m.average_battle_score(),
                m.harvested_credits,
                m.income_per_minute(),
                m.opening_income_per_minute(),
                m.income_games,
                average(m.spent as f64, m.games),
                average(m.final_funds as f64, m.games),
                average(m.kills as f64, m.games),
                average(m.losses as f64, m.games),
                average(m.built as f64, m.games),
                average(m.low_power_ticks as f64, m.ticks) * 100.0,
                average(m.rejected as f64, m.submitted) * 100.0,
                m.submitted,
                m.dropped
            ));
        }
    }
    csv
}
