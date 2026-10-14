import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const results = path.join(root, 'arena-results/improvements');
const destination = path.join(root, 'projects/tooling/ra-arena');
const read = (name) => JSON.parse(fs.readFileSync(path.join(results, name), 'utf8'));
const sha256 = (file) => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const seeds = [20261014, 8675309, 314159];
const holdouts = seeds.map((seed) => read(`holdout-${seed}.json`));

function gamesFor(report, npc) {
    return report.matches.flatMap((m) => {
        const seat = m.spec.seats.indexOf(npc);
        if (seat < 0) return [];
        const score = seat === 0 || m.score === 'draw' ? m.score : m.score === 'win' ? 'loss' : 'win';
        return [{ opponent: m.spec.seats[1 - seat], scenario: m.spec.scenario, score }];
    });
}

function tally(games) {
    const wins = games.filter((g) => g.score === 'win').length;
    const draws = games.filter((g) => g.score === 'draw').length;
    const losses = games.length - wins - draws;
    return { games: games.length, wins, draws, losses, score_rate: (wins + draws / 2) / games.length };
}

function sumMetrics(metrics) {
    const totals = {};
    for (const m of metrics) {
        for (const [key, value] of Object.entries(m)) totals[key] = (totals[key] ?? 0) + value;
    }
    return {
        ...totals,
        income_per_game_minute: (totals.harvested_credits * 900) / totals.ticks,
        opening_income_per_game_minute: (totals.opening_harvested * 900) / totals.opening_ticks,
        first_income_seconds: totals.income_games ? totals.first_income_ticks / totals.income_games / 15 : null,
        average_battle_score: totals.battle_score / totals.games,
        rejected_pct: totals.submitted ? (totals.rejected * 100) / totals.submitted : 0,
    };
}

const allGames = holdouts.flatMap((r) => gamesFor(r, 'candidate'));
const opponentIds = holdouts[0].config.npcs.filter((id) => id !== 'candidate');
const byOpponent = Object.fromEntries(opponentIds.map((id) => [id, tally(allGames.filter((g) => g.opponent === id))]));
const byScenario = Object.fromEntries(
    holdouts[0].config.scenarios.map((scene) => [
        scene,
        {
            ...tally(allGames.filter((g) => g.scenario === scene)),
            metrics: sumMetrics(holdouts.map((r) => r.summary.metrics_by_scenario[scene].candidate)),
        },
    ]),
);
const plateauReports = [42, 137].map((seed) => read(`11-plateau-${seed}.json`));
const plateau = plateauReports[0].config.npcs
    .filter((id) => id !== 'reference-v7')
    .map((id) => ({
        npc: id,
        ...tally(plateauReports.flatMap((r) => gamesFor(r, id)).filter((g) => g.opponent === 'reference-v7')),
    }));
const trainingFiles = fs.readdirSync(results).filter((n) => /^\d\d-.*\.json$/.test(n));
const training = trainingFiles.map((file) => ({ file, report: read(file) })).filter((r) => r.report.summary?.games);
const trainingGames = training.reduce((sum, r) => sum + r.report.summary.games, 0);
const holdoutGames = holdouts.reduce((sum, r) => sum + r.summary.games, 0);
const source = path.join(root, 'projects/engine/ra-engine/src/gameplay/npc/candidate.rs');
if (sha256(source) !== sha256(path.join(results, 'final-candidate.rs'))) throw new Error('候选源码与验收快照不同，请重跑验收');
const summary = {
    date: '2026-10-14',
    protocol: holdouts[0].protocol,
    metrics_version: holdouts[0].metrics_version,
    rules_hash: holdouts[0].rules_hash,
    candidate_sha256: sha256(source),
    validation_binary_sha256: sha256(path.join(results, 'final.exe')),
    training_seeds: [42, 137],
    tested_variants: 41,
    training_games: trainingGames,
    holdout_seeds: seeds,
    holdout_games: holdoutGames,
    total_experiment_games: trainingGames + holdoutGames,
    candidate: tally(allGames),
    by_opponent: byOpponent,
    by_scenario: byScenario,
    ratings_by_seed: holdouts.map((r) => ({
        seed: r.config.seed,
        candidate: r.leaderboard.find((x) => x.npc === 'candidate'),
        original: r.leaderboard.find((x) => x.npc === 'hard-baseline-v1'),
    })),
    plateau_challengers: plateau,
    raw_reports: [...training.map((r) => r.file), ...seeds.map((seed) => `holdout-${seed}.json`)].map((file) => ({
        file,
        sha256: sha256(path.join(results, file)),
    })),
    limits: '全图信息、Rust 合成规则、三类对称场景；新种子仍可能覆盖已见地图几何。局部瓶颈不代表原版 RA2 最优或统计显著性。',
};
fs.writeFileSync(path.join(destination, 'optimization-results-v1.json'), `${JSON.stringify(summary, null, 4)}\n`);

const pct = (v) => `${(v * 100).toFixed(2)}%`;
let text = `# NPC 策略迭代结果（2026-10-14）\n\n当前候选在三个场景中，对原始 Hard 的保留种子验收为 **${byOpponent['hard-baseline-v1'].wins}/${byOpponent['hard-baseline-v1'].games} 胜**。共完成 ${summary.total_experiment_games} 局策略实验和验收，其中调参 ${trainingGames} 局、三个保留种子验收 ${holdoutGames} 局，候选实际参加验收 ${allGames.length} 局。\n\n`;
text +=
    '候选代码在 [`candidate.rs`](../../engine/ra-engine/src/gameplay/npc/candidate.rs)，可执行的各代对手及其 SHA-256 在 [champions-v1.json](baselines/champions-v1.json)。完整机器可读结果在 [optimization-results-v1.json](optimization-results-v1.json)。\n\n';
text +=
    '## 保留种子验收\n\n种子 `20261014`、`8675309`、`314159`；同一六人参赛池、三个场景、每对每场景 20 轮镜像双局、3000 tick 上限、64 命令/拍。每个种子 1800 局。候选和 `reference-v7` 是同一策略的两份独立源码副本（在当前规则下行为相同），用于保持固定的最近强版本参照。\n\n';
text += '| 对手 | 局数 | 胜 | 和 | 负 | 平均对局分 |\n|---|---:|---:|---:|---:|---:|\n';
for (const [id, row] of Object.entries(byOpponent))
    text += `| ${id} | ${row.games} | ${row.wins} | ${row.draws} | ${row.losses} | ${row.score_rate.toFixed(4)} |\n`;
text += '\nElo 仅在这个固定参赛池中解读：\n\n| 主种子 | 候选 Elo | 原始 Hard Elo |\n|---|---:|---:|\n';
for (const r of summary.ratings_by_seed) text += `| ${r.seed} | ${r.candidate.elo.toFixed(2)} | ${r.original.elo.toFixed(2)} |\n`;
text += '\n候选仍会输给强版本和反制策略；目标是提高多对手、多场景表现，不能把击败原始 Hard 当成全局最强。不同轮次或参赛池的 Elo 不作差。\n\n';
text += '| 场景 | 平均对局分 | 采矿/游戏分钟 | 前60秒采矿/分钟 | 首次收入秒 | 平均游戏积分 | 拒绝率 |\n|---|---:|---:|---:|---:|---:|---:|\n';
for (const [scene, r] of Object.entries(byScenario)) {
    const m = r.metrics;
    text += `| ${scene} | ${r.score_rate.toFixed(4)} | ${m.income_per_game_minute.toFixed(2)} | ${m.opening_income_per_game_minute.toFixed(2)} | ${m.first_income_seconds?.toFixed(2) ?? '—'} | ${m.average_battle_score.toFixed(2)} | ${m.rejected_pct.toFixed(3)}% |\n`;
}
text += '\n## 保留的改进\n\n';
text += '- 建造命令串行决策，同拍资金统一分配；军队上限同时计算存活和在建单位，普通量产排除矿车与无武器单位。\n';
text += '- 去掉固定 45 tick 等待和盲目重复下单，按空闲工厂持续生产；按实际电力缺口补电。\n';
text += '- 矿场按矿区距离和完整占地选择落点，避免覆盖矿石；首次收入从原策略约 5.87 秒提前到约 4.93 秒。\n';
text += '- 纯车辆协同接敌，混编部队优先处理射程内威胁；按伤害、目标护甲与敌方输出选择目标，并分配本拍伤害以减少过量集火。\n';
text += '- 利用射程优势拉扯，混编部队在低于一半血量时撤退；纯车辆阵形保留原来的协同方式，避免一律撤退导致战斗场景退步。\n';
text += '- 存在持续矿源时适当前置生产建筑；有限资金场保持紧凑基地，保留步兵与载具混编。\n\n';
text +=
    '## 局部瓶颈的依据\n\n训练种子为 `42` 和 `137`。先检查单项变化，再测试组合，最后在当前冻结版本周围测试 11 种相邻配置。下面均从挑战者视角统计它与 `reference-v7` 的直接交手，每项在两个种子合计 48 局；没有一项得分率超过 50%。\n\n';
text += '| 相邻配置 | 局数 | 胜 | 和 | 负 | 得分率 |\n|---|---:|---:|---:|---:|---:|\n';
for (const row of plateau) text += `| ${row.npc} | ${row.games} | ${row.wins} | ${row.draws} | ${row.losses} | ${pct(row.score_rate)} |\n`;
text +=
    '\n更早还试过 8/16/64 单位上限、纯步兵/纯车辆、双矿车、双矿场、车厂优先、不同协同距离、队列加深、火力分配方式和无条件撤退。部分方案被当前方案稳定压制，部分与当前方案打平，部分存在场景之间的取舍。所有 41 个调参 ID 仍可用 CLI 的 `--bots tune-...` 复跑；其参数起点固定，不随候选默认配置改变。\n\n';
text +=
    '停止继续调参的结论是：当前实现的这些策略方向，在现有场景池中出现局部瓶颈。并未证明全局最优；未尝试的算法、地形或对手仍可能带来提升。三个保留主种子没有用于这轮调参，但地图的变化范围有限，会与训练阶段重复，不应将局数当成同样数量的独立地图样本。下一阶段应扩展地图、兵种克制、长局经济和战争迷雾。\n\n';
text +=
    '## 复跑\n\n```powershell\n.\\test-npc.cmd\n# 完整回归 + release 构建，并复跑其中一个保留种子\n.\\test-npc.cmd --full --rounds 20 --seed 20261014\n# 单独复查一个相邻配置与冻结版本\ncargo run --profile arena -p ra-arena -- run --bots tune-health25,reference-v7 --scenarios duel-v1,skirmish-v1,economy-v1 --rounds 20 --seed 42 --jobs 4 --out arena-results/health-check.json\n```\n\n';
text += `候选 SHA-256：\`${summary.candidate_sha256}\`。验收二进制与候选快照保存在本地 \`arena-results/improvements/final.exe\` 和 \`final-candidate.rs\`。\n\n`;
text += seeds.map((seed) => `- [种子 ${seed} 的原始报告](../../../arena-results/improvements/holdout-${seed}.md)`).join('\n');
text +=
    '\n\n本报告由 `node scripts/npc/summarize-experiments.mjs` 从原始 JSON 汇总；不会修改比赛记录中的 64 位种子。指标定义见 [npc-workflow.md](npc-workflow.md)。\n';
fs.writeFileSync(path.join(destination, 'optimization-report.md'), text);
console.log(JSON.stringify({ total_games: summary.total_experiment_games, candidate: summary.candidate, by_opponent: byOpponent }, null, 2));
