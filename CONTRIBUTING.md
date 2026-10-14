# 参与 rust-ra2 研究分支

此仓库是 ra2-agent 社区的引擎与评测工具分支，来源为 [rust-alert/ra2.exe](https://github.com/rust-alert/ra2.exe)，默认开发分支为 `ra2-agent`。请保留原有版权与许可。

## 改动范围

- 引擎、Agent 接口、回放与性能工具改动提交到本仓库。
- 社区策略源码按 [agents 的贡献规范](https://github.com/ra2-agent/agents/blob/main/CONTRIBUTING.md) 提交到 agents 仓库。
- 正式比赛由维护者测试并录入 leaderboard；自测和本仓库实验报告不自动成为社区正式 Elo。

## 验证

```sh
cargo test --locked -p ra-ecs -p ra-types -p ra-adaptor -p ra-engine -p ra-testing -p ra-arena
cargo test --locked -p ra-engine -p ra-arena --features ra-arena/profiling
node --test scripts/npc/report.test.mjs
```

修改策略时使用新的版本 ID，不覆盖冻结 Hard 和历史参照。变更玩法时说明金样和回放兼容影响。性能优化说明固定赛程、是否启用计时插桩、CPU/墙钟/分配/内存的测量口径，比较命令、胜负与终局哈希。

正式评测固定完整源码提交、Cargo.lock、编译器版本、场景规则和 Agent 指纹；保持干净工作区。原版资源、凭据、个人安装目录、构建产物与 arena-results 不提交到 Git。CI 运行回归测试，完整 NPC evaluation 仅由维护者手动触发。
