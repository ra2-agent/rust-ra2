import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { compareReports, comparisonMarkdown, fingerprint, metricRows } from './npc/report.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const storage = path.join(root, 'arena-results/npc');
const candidateSource = 'projects/engine/ra-engine/src/gameplay/npc/candidate.rs';
const configPath = 'projects/tooling/ra-arena/npc-test.json';
const baselineManifestPath = 'projects/tooling/ra-arena/baselines/hard-v1/manifest.json';
const championManifestPath = 'projects/tooling/ra-arena/baselines/champions-v1.json';
const readJson = (file) => JSON.parse(fs.readFileSync(file, 'utf8'));
const sha256 = (file) => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const writeJson = (file, data) => fs.writeFileSync(file, `${JSON.stringify(data, null, 2)}\n`);
const portable = (file) => file.replaceAll('\\', '/');
const steps = [];
let output;

function options(args) {
    const result = { full: false, label: 'candidate', compare: undefined, overrides: {} };
    for (let i = 0; i < args.length; i++) {
        const flag = args[i];
        if (flag === '--full') {
            result.full = true;
            continue;
        }
        if (flag === '--help' || flag === '-h') return null;
        if (!['--rounds', '--seed', '--jobs', '--max-ticks', '--label', '--compare'].includes(flag)) throw new Error(`未知参数 ${flag}`);
        const value = args[++i];
        if (!value || value.startsWith('--')) throw new Error(`${flag} 缺少值`);
        if (flag === '--label') {
            if (!/^[a-zA-Z0-9_-]{1,40}$/.test(value)) throw new Error('label 仅允许 1–40 个字母、数字、-、_');
            result.label = value;
        } else if (flag === '--compare') {
            result.compare = path.resolve(root, value);
        } else {
            const number = Number(value);
            if (!Number.isSafeInteger(number) || number < (flag === '--seed' ? 0 : 1)) throw new Error(`${flag} 需要有效整数`);
            result.overrides[flag] = value;
        }
    }
    return result;
}

function git(args) {
    const result = spawnSync('git', args, { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024, windowsHide: true });
    if (result.error || result.status !== 0) throw new Error(`git ${args[0]} 失败：${result.error?.message ?? result.stderr}`);
    return result.stdout;
}

function sourceFiles() {
    const files = git(['ls-files', '--cached', '--others', '--exclude-standard', '-z']).split('\0').filter(Boolean);
    // 备份构建输入和测试资源；无窗口评测不需要 sites、平台分发包或 node_modules。
    const selected = files.filter(
        (name) =>
            /^(projects\/(engine|adapters|bindings|tooling)\/|scripts\/|\.github\/)/.test(name) ||
            ['Cargo.toml', 'rust-toolchain.toml', 'rustfmt.toml', 'package.json', 'biome.json'].includes(name),
    );
    if (fs.existsSync(path.join(root, 'Cargo.lock'))) selected.push('Cargo.lock');
    return [...new Set(selected)].sort().filter((name) => fs.statSync(path.join(root, name)).isFile());
}

function sharedHashes(files) {
    return Object.fromEntries(
        Object.entries(files).filter(
            ([name]) =>
                name !== candidateSource &&
                (/^projects\/(engine|adapters)\/[^/]+\/(src\/|data\/|Cargo.toml)/.test(name) ||
                    /^projects\/tooling\/ra-arena\/(src\/|fixtures\/.*\.ini$|Cargo.toml$)/.test(name) ||
                    ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'scripts/npc/report.mjs'].includes(name)),
        ),
    );
}

function runStep(name, command, args) {
    console.log(`[${steps.length + 1}] ${name}`);
    const started = performance.now();
    const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024, windowsHide: true });
    const log = path.join(output, 'logs', `${steps.length + 1}.log`);
    const text = `${result.stdout ?? ''}${result.stderr ?? ''}${result.error ? `\n${result.error.message}\n` : ''}`;
    fs.writeFileSync(log, text);
    steps.push({ name, seconds: (performance.now() - started) / 1000, exit_code: result.status, log: path.relative(output, log) });
    writeJson(path.join(output, 'steps.json'), steps);
    if (result.error || result.status !== 0) {
        throw new Error(`${name} 失败，退出码 ${result.status}。\n${text.split('\n').slice(-35).join('\n')}\n日志：${log}`);
    }
    return result.stdout;
}

function cargoBinary() {
    const executable = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
    const installed = path.join(os.homedir(), '.cargo', 'bin', executable);
    if (fs.existsSync(installed)) return installed;
    return executable;
}

function main() {
    const opts = options(process.argv.slice(2));
    if (!opts) {
        console.log(
            '一键 NPC 测试：node scripts/npc-test.mjs [--full] [--rounds N] [--seed N] [--jobs N] [--max-ticks N] [--label NAME] [--compare 历史运行目录]\n默认用 arena 优化增量构建，先测试再运行镜像联赛和诊断回放，导出 JSON/Markdown/CSV，自动比较上次同赛程记录。--full 使用 release、扩大相关 crate 测试，并默认评测 30 轮。',
        );
        return;
    }
    const baseline = readJson(path.join(root, baselineManifestPath));
    const champions = readJson(path.join(root, championManifestPath));
    for (const [name, expected] of Object.entries({ ...baseline.files, ...champions.files })) {
        if (sha256(path.join(root, name)) !== expected)
            throw new Error(`冻结基线发生改动：${name}。请在 candidate.rs 修改策略；如需新基线，请另建版本。`);
    }
    const latestPath = path.join(storage, 'latest.json');
    const previousRun = opts.compare ?? (fs.existsSync(latestPath) ? path.resolve(root, readJson(latestPath).directory) : undefined);
    if (previousRun && (!fs.existsSync(path.join(previousRun, 'league.json')) || !fs.existsSync(path.join(previousRun, 'manifest.json')))) {
        throw new Error(`历史目录缺少 league.json 或 manifest.json：${previousRun}`);
    }
    const localTime = new Intl.DateTimeFormat('sv-SE', { timeZone: 'Asia/Shanghai', dateStyle: 'short', timeStyle: 'medium' }).format(
        new Date(),
    );
    const stamp = `${localTime.replaceAll(/[-: ]/g, '')}-${Date.now() % 1000}`;
    output = path.join(storage, `${stamp}-${opts.label}`);
    fs.mkdirSync(output, { recursive: false });
    fs.mkdirSync(path.join(output, 'logs'));
    console.log(`本次记录：${output}`);

    const files = Object.fromEntries(sourceFiles().map((name) => [name, sha256(path.join(root, name))]));
    const manifest = {
        status: 'running',
        created_at: new Date().toISOString(),
        created_at_shanghai: localTime,
        source_revision: git(['rev-parse', 'HEAD']).trim(),
        source_dirty: Boolean(git(['status', '--porcelain']).trim()),
        label: opts.label,
        source_hash: fingerprint(files),
        shared_source_hash: fingerprint(sharedHashes(files)),
        candidate_sha256: files[candidateSource],
        frozen_baseline: baseline,
        frozen_champions: champions,
        build_profile: opts.full ? 'release' : 'arena',
        files,
    };
    for (const name of Object.keys(files)) {
        const destination = path.join(output, 'source', name);
        fs.mkdirSync(path.dirname(destination), { recursive: true });
        fs.copyFileSync(path.join(root, name), destination);
    }
    fs.writeFileSync(path.join(output, 'workspace.patch'), git(['diff', '--binary', 'HEAD']));
    writeJson(path.join(output, 'manifest.json'), manifest);
    const config = readJson(path.join(root, configPath));
    if (opts.full) config.rounds = 30;
    for (const [flag, key] of [
        ['--rounds', 'rounds'],
        ['--seed', 'seed'],
        ['--jobs', 'jobs'],
        ['--max-ticks', 'max_ticks'],
    ]) {
        if (opts.overrides[flag]) config[key] = Number(opts.overrides[flag]);
    }
    writeJson(path.join(output, 'config.json'), config);

    runStep('报告对比逻辑测试', process.execPath, ['--test', 'scripts/npc/report.test.mjs']);
    const cargo = cargoBinary();
    manifest.rustc = runStep('记录编译器版本', path.join(path.dirname(cargo), process.platform === 'win32' ? 'rustc.exe' : 'rustc'), [
        '--version',
    ]).trim();
    manifest.cargo = runStep('记录 Cargo 版本', cargo, ['--version']).trim();
    const profileArgs = ['--profile', manifest.build_profile];
    runStep('Rust 回归测试', cargo, [
        'test',
        '--locked',
        ...profileArgs,
        '-p',
        'ra-arena',
        ...(opts.full ? ['-p', 'ra-engine', '-p', 'ra-adaptor', '-p', 'ra-testing', '-p', 'ra-types', '-p', 'ra-ecs'] : []),
    ]);
    if (!opts.full)
        runStep('矿车收入记账测试', cargo, ['test', '--locked', ...profileArgs, '-p', 'ra-engine', 'gameplay::economy::ore_income']);
    runStep(`构建 ${manifest.build_profile} NPC 评测器`, cargo, ['build', '--locked', ...profileArgs, '-p', 'ra-arena']);
    const metadata = JSON.parse(runStep('读取构建目录', cargo, ['metadata', '--locked', '--format-version', '1', '--no-deps']));
    const binary = path.join(metadata.target_directory, manifest.build_profile, process.platform === 'win32' ? 'ra-arena.exe' : 'ra-arena');
    manifest.binary_sha256 = sha256(binary);
    const leaguePath = path.join(output, 'league.json');
    runStep('固定种子镜像联赛', binary, ['run', '--config', path.join(output, 'config.json'), '--out', leaguePath]);
    const replayPath = path.join(output, 'replay.json');
    runStep('保存候选版与 Hard 的诊断轨迹', binary, [
        'run',
        '--bots',
        'candidate,hard-baseline-v1',
        '--scenarios',
        config.scenarios.join(','),
        '--rounds',
        '1',
        '--seed',
        `${config.seed}`,
        '--max-ticks',
        `${config.max_ticks}`,
        '--max-commands',
        `${config.max_commands_per_tick}`,
        '--jobs',
        `${config.jobs}`,
        '--trace',
        '--out',
        replayPath,
    ]);
    runStep('回放校验命令、胜负、状态哈希和指标', binary, ['replay', replayPath]);

    const finalFiles = Object.fromEntries(sourceFiles().map((name) => [name, sha256(path.join(root, name))]));
    if (fingerprint(finalFiles) !== manifest.source_hash || sha256(binary) !== manifest.binary_sha256) {
        const changed = [...new Set([...Object.keys(files), ...Object.keys(finalFiles)])].filter((name) => files[name] !== finalFiles[name]);
        if (sha256(binary) !== manifest.binary_sha256) changed.push('评测二进制');
        throw new Error(`评测期间源码或二进制发生变化，本次不更新历史参照。请在改动稳定后重跑。\n${changed.slice(0, 12).join('\n')}`);
    }
    const report = readJson(leaguePath);
    const comparison = previousRun
        ? compareReports(readJson(path.join(previousRun, 'league.json')), report, readJson(path.join(previousRun, 'manifest.json')), manifest)
        : undefined;
    const comparisonText = comparisonMarkdown(comparison, previousRun ? portable(path.relative(root, previousRun)) : '');
    writeJson(path.join(output, 'comparison.json'), comparison ?? { compatible: false, reasons: ['首次运行'], rows: [] });
    fs.writeFileSync(path.join(output, 'comparison.md'), comparisonText);
    const baselineRows = metricRows(report, 'hard-baseline-v1');
    const candidateRows = metricRows(report);
    writeJson(path.join(output, 'candidate.json'), {
        candidate: candidateRows,
        hard_baseline: baselineRows,
        head_to_head: report.summary.head_to_head.candidate['hard-baseline-v1'],
        comparison,
    });
    fs.mkdirSync(path.join(output, 'bin'));
    fs.copyFileSync(binary, path.join(output, 'bin', path.basename(binary)));
    manifest.status = 'passed';
    manifest.completed_at = new Date().toISOString();
    writeJson(path.join(output, 'manifest.json'), manifest);
    fs.writeFileSync(
        path.join(output, 'index.md'),
        `${fs.readFileSync(path.join(output, 'league.md'), 'utf8')}\n${comparisonText}\n源码快照：[source](source/)；构建输入指纹：[manifest.json](manifest.json)；测试日志：[logs](logs/)；回放：[replay.json](replay.json)。\n`,
    );
    const temporaryLatest = path.join(storage, `latest-${path.basename(output)}.tmp`);
    writeJson(temporaryLatest, { directory: portable(path.relative(root, output)), candidate_sha256: manifest.candidate_sha256 });
    fs.renameSync(temporaryLatest, latestPath);
    console.table(
        candidateRows.map((r) => ({
            scene: r.scenario,
            Elo: r.elo.toFixed(1),
            points: r.avg_match_points.toFixed(3),
            score: r.avg_battle_score.toFixed(2),
            'income/min': r.income_per_game_minute.toFixed(2),
            'opening/min': r.opening_income_per_game_minute.toFixed(2),
            'first income/s': r.first_income_seconds?.toFixed(2) ?? '—',
        })),
    );
    console.log(comparisonText);
    console.log(
        `全部通过。报告：${path.join(output, 'index.md')}\nCSV：${path.join(output, 'league.csv')}\n之后修改 ${candidateSource}，再次运行同一命令即可。`,
    );
}

try {
    fs.mkdirSync(storage, { recursive: true });
    main();
} catch (error) {
    if (output) {
        writeJson(path.join(output, 'failure.json'), { status: 'failed', error: error.message, steps });
        const manifestPath = path.join(output, 'manifest.json');
        if (fs.existsSync(manifestPath)) writeJson(manifestPath, { ...readJson(manifestPath), status: 'failed' });
    }
    console.error(`NPC 测试失败：${error.message}`);
    process.exitCode = 1;
}
