import assert from 'node:assert/strict';
import { test } from 'node:test';
import { compareReports, fingerprint, metricRows } from './report.mjs';

function report() {
    return {
        protocol: 'full-info-v1',
        metrics_version: 'metrics-v1',
        rules_hash: 'r',
        engine_version: '1',
        config: {
            npcs: ['candidate', 'hard-baseline-v1'],
            scenarios: ['economy-v1'],
            seed: 42,
            rounds: 4,
            max_ticks: 3000,
            max_commands_per_tick: 64,
            elo: { initial: 1500, k: 32, scale: 400 },
            jobs: 1,
        },
        leaderboard: [{ npc: 'candidate', elo: 1500, wins: 1, draws: 2, losses: 1 }],
        by_scenario: {},
        summary: {
            metrics: {
                candidate: {
                    games: 4,
                    battle_score: 400,
                    harvested_credits: 1000,
                    ticks: 1800,
                    opening_harvested: 200,
                    opening_ticks: 900,
                    income_games: 2,
                    first_income_ticks: 300,
                    spent: 2000,
                    rejected: 2,
                    submitted: 100,
                    dropped: 0,
                },
            },
        },
    };
}

const manifest = { shared_source_hash: 'same-shared-engine' };

test('income uses summed durations, score and first-income denominators stay distinct', () => {
    const row = metricRows(report())[0];
    assert.equal(row.avg_match_points, 0.5);
    assert.equal(row.avg_battle_score, 100);
    assert.equal(row.income_per_game_minute, 500);
    assert.equal(row.first_income_seconds, 10);
    assert.equal(row.rejected_pct, 2);
});

test('no-income is missing first arrival, legacy metrics are unknown', () => {
    const r = report();
    r.summary.metrics.candidate.income_games = 0;
    assert.equal(metricRows(r)[0].first_income_seconds, null);
    delete r.summary;
    assert.deepEqual(metricRows(r), []);
});

test('only candidate change and concurrency change allow paired history comparison', () => {
    const before = report();
    const after = report();
    after.config.jobs = 4;
    after.leaderboard[0].elo += 16;
    after.summary.metrics.candidate.battle_score += 40;
    const comparison = compareReports(before, after, manifest, manifest);
    assert.equal(comparison.compatible, true);
    assert.equal(comparison.rows[0].delta.elo, 16);
    assert.equal(comparison.rows[0].delta.avg_battle_score, 10);
});

test('different seeds, opponent order, rules or shared engine refuse attribution', () => {
    for (const mutate of [
        (r) => {
            r.config.seed += 1;
        },
        (r) => {
            r.config.npcs.reverse();
        },
        (r) => {
            r.rules_hash = 'changed';
        },
        (r) => {
            delete r.metrics_version;
        },
    ]) {
        const after = report();
        mutate(after);
        assert.equal(compareReports(report(), after, manifest, manifest).compatible, false);
    }
    assert.equal(compareReports(report(), report(), manifest, { shared_source_hash: 'different' }).compatible, false);
});

test('identical input has zero deltas; fingerprints include filenames and are order-independent', () => {
    assert.equal(compareReports(report(), report(), manifest, manifest).rows[0].delta.elo, 0);
    assert.equal(fingerprint({ b: '2', a: '1' }), fingerprint({ a: '1', b: '2' }));
    assert.notEqual(fingerprint({ a: '1' }), fingerprint({ b: '1' }));
});
