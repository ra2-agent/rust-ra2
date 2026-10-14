import { createHash } from 'node:crypto';

export function fingerprint(files) {
    const hash = createHash('sha256');
    for (const name of Object.keys(files).sort()) hash.update(`${name}\0${files[name]}\n`);
    return hash.digest('hex');
}

const average = (sum, count) => (count > 0 ? sum / count : 0);

export function metricRows(report, npc = 'candidate') {
    const scopes = [['overall', report.leaderboard, report.summary?.metrics]];
    for (const [scene, ratings] of Object.entries(report.by_scenario)) {
        scopes.push([scene, ratings, report.summary?.metrics_by_scenario?.[scene]]);
    }
    return scopes.flatMap(([scenario, ratings, metrics]) => {
        const row = ratings.find((r) => r.npc === npc);
        const m = metrics?.[npc];
        if (!row || !m?.games) return [];
        const games = row.wins + row.draws + row.losses;
        return [
            {
                scenario,
                npc,
                games,
                elo: row.elo,
                avg_match_points: average(row.wins + 0.5 * row.draws, games),
                avg_battle_score: average(m.battle_score, m.games),
                income_per_game_minute: average(m.harvested_credits * 900, m.ticks),
                opening_income_per_game_minute: average(m.opening_harvested * 900, m.opening_ticks),
                first_income_seconds: m.income_games ? m.first_income_ticks / m.income_games / 15 : null,
                income_games: m.income_games,
                avg_spent: average(m.spent, m.games),
                rejected_pct: average(m.rejected * 100, m.submitted),
                dropped: m.dropped,
            },
        ];
    });
}

export function compareReports(previous, current, previousManifest, currentManifest) {
    const reasons = [];
    for (const key of ['protocol', 'metrics_version', 'rules_hash', 'engine_version']) {
        if (!previous[key] || previous[key] !== current[key]) reasons.push(`${key} 不一致或缺失`);
    }
    // 并发和存储选项不改变赛程；NPC 顺序、种子、预算和 Elo 参数必须一致。
    for (const key of ['npcs', 'scenarios', 'rounds', 'seed', 'max_ticks', 'max_commands_per_tick', 'elo']) {
        if (JSON.stringify(previous.config[key]) !== JSON.stringify(current.config[key])) reasons.push(`赛程配置 ${key} 不一致`);
    }
    if (!previousManifest?.shared_source_hash || previousManifest.shared_source_hash !== currentManifest?.shared_source_hash) {
        reasons.push('共享引擎、规则或评测实现的源码指纹不同，不能将变化归因于候选策略');
    }
    if (!previous.summary?.metrics?.candidate || !current.summary?.metrics?.candidate) reasons.push('缺少 candidate 遥测');
    if (reasons.length) return { compatible: false, reasons, rows: [] };
    const before = new Map(metricRows(previous).map((r) => [r.scenario, r]));
    const rows = metricRows(current).map((after) => {
        const old = before.get(after.scenario);
        const delta = {};
        for (const key of [
            'elo',
            'avg_match_points',
            'avg_battle_score',
            'income_per_game_minute',
            'opening_income_per_game_minute',
            'rejected_pct',
        ]) {
            delta[key] = after[key] - old[key];
        }
        delta.first_income_seconds =
            old.first_income_seconds === null || after.first_income_seconds === null
                ? null
                : after.first_income_seconds - old.first_income_seconds;
        return { scenario: after.scenario, before: old, after, delta };
    });
    return { compatible: true, reasons: [], rows };
}

const number = (value) => (value === null ? '—' : value.toFixed(3));
const signed = (value) => (value === null ? '—' : `${value >= 0 ? '+' : ''}${number(value)}`);

export function comparisonMarkdown(comparison, previousRun) {
    if (!comparison) return '# 历史对比\n\n首次运行，已建立初始记录。\n';
    let text = `# 与上次候选版比较\n\n上一记录：${previousRun}\n\n`;
    if (!comparison.compatible) return `${text}未计算差值：\n\n${comparison.reasons.map((r) => `- ${r}`).join('\n')}\n`;
    text += '| 场景 | Δ Elo | Δ 平均对局分 | Δ 游戏积分 | Δ 采矿/游戏分钟 | Δ 前60秒采矿/分钟 | Δ 首次收入秒 | Δ 拒绝率百分点 |\n';
    text += '|---|---:|---:|---:|---:|---:|---:|---:|\n';
    for (const row of comparison.rows) {
        const d = row.delta;
        text += `| ${row.scenario} | ${signed(d.elo)} | ${signed(d.avg_match_points)} | ${signed(d.avg_battle_score)} | ${signed(d.income_per_game_minute)} | ${signed(d.opening_income_per_game_minute)} | ${signed(d.first_income_seconds)} | ${signed(d.rejected_pct)} |\n`;
    }
    return `${text}\n正的胜负得分差表示此固定样本中改善；首次收入秒和拒绝率越低越好。差值是固定赛程的观察结果，不是统计显著性结论。改变种子、赛程或共享引擎后需重新建立参照。\n`;
}
