# 五子棋引擎与本地擂台

独立的 Rust 引擎，规则为 15×15、无禁手、五连及以上获胜。搜索包括增量棋型评估、迭代加深、Alpha-Beta、置换表，以及 VCF 和 VCT。只有独立算杀求解完成才会把 `proven` 标为 `true`。搜索估分是引擎内部单位。

## 使用

- `gomoku.exe` 是 Windows 命令行引擎，通过 GTP 从标准输入接收指令。
- 双击打开 `棋谱回放.html`，再选择 `results` 中的 JSONL 棋谱。可拖动进度条或用左右方向键逐手回放。
- `arena.mjs` 使用 Node.js 22 或更新版本，无需安装第三方包。默认读取桌面上的对手文件，也可用 `--desktop` 指定桌面目录。

在此目录下运行：

```powershell
@('boardsize 15', 'time_ms 1000', 'play B H8', 'genmove W', 'last_search', 'quit') | .\gomoku.exe
node arena.mjs --opponent vct --set test --pairs 4 --ms 1000
node arena.mjs --opponent hybrid --set test --pairs 1 --ms 5000
node arena.mjs --opponent katagomo --set test --pairs 1 --ms 300 --wall-ms 2000
cargo test --offline
cargo build --offline --release
```

PowerShell 5.1 会给管道首行加 UTF-8 BOM。引擎现在会剥掉行首的 BOM，所以上面这条命令的第一行也能正常识别；换作其他 GTP 客户端时不需要这一处理。

## 实测结果

以下战绩均为新引擎视角。1 秒档用四组开局，每组换边。5 秒档复测其中一组。

| 对手 | 每步目标 | 正常完赛 | 其他结果 | 棋谱 |
| --- | ---: | --- | --- | --- |
| VCT | 1 秒 | 8 胜 0 和 0 负 |  | [VCT 1 秒档](results/vct-1s.jsonl) |
| 双脑混合版 | 1 秒 | 6 胜 0 和 2 负 |  | [双脑版 1 秒档](results/hybrid-1s.jsonl) |
| VCT | 5 秒 | 2 胜 0 和 0 负 |  | [VCT 5 秒档](results/vct-5s.jsonl) |
| 双脑混合版 | 5 秒 | 0 局 | 对手两盘均返回了已占用坐标 | [双脑版 5 秒档](results/hybrid-5s.jsonl) |
| Katagomo 40Block | 0.3 秒 | 0 胜 0 和 2 负 |  | [Katagomo 短赛](results/katagomo-300ms.jsonl) |

双脑版在 1 秒档两盘胜局都执黑。新引擎在两盘分别于第 12 手和第 16 手首次搜索到接近必败的估分，适合从开局和中盘交界处回放。Katagomo 两盘平均每手实际耗时约 382 毫秒，新引擎约 152 毫秒。Katagomo 此前有一组对局因 390 毫秒的裁判上限过早结束，表中采用放宽裁判上限后正常完赛的一组。

## 对战条件

- 每组用同一个指定开局下两盘，新引擎交换黑白。短时限调试用 `dev` 开局，报告采用独立的 `test` 开局。5 秒复测复用其中第一组。
- VCT 和双脑版使用原文件中的算法。擂台在内存中限制两者的阶段计时和总预算，支持用 `--native` 按原生时钟运行。
- Katagomo 使用桌面的 `freestyle.exe` 和原 40Block 模型，通过本地 GTP 运行并关闭后台思考。0.3 秒档是目标用时，实际落子时间在棋谱里，复测的裁判上限为 2 秒。
- 棋谱里的 `candidateColor` 为 1 时新引擎执黑，2 时执白。`ms` 是裁判实测的落子耗时。早期对局的部分搜索日志受标准输出和标准错误到达顺序影响，有缺字，落子和计时记录完整。

## 依赖的对手文件

擂台默认从同一台机器的桌面读取对手，路径写死在 `arena.mjs` 里：

| 对手 | 路径 |
| --- | --- |
| VCT | `Desktop\博弈\棋类\五子棋\算法核心与Katagomo超算SDK\vct_engine.js` |
| 双脑混合版 | `Desktop\博弈\棋类\五子棋\五子棋_终极AI_双脑混合版.html` |
| Katagomo 40Block | `Desktop\游戏\Katagomo_Lizzieyzy_Package_20210520\` |

这些文件不在本仓库内，换机器复跑需要自备，或用 `--desktop` 指向自己的桌面目录。Katagomo 那一组还需要 `freestyle.exe` 和 `freestyle.bin.gz` 模型。

## 已知问题

- 双脑混合版在 5 秒档会返回已占用的坐标（日志里 `reason=illegal-move`），那是对手自身的状态问题，不是本引擎造成的。该组两盘不计入正常完赛。
- 早期棋谱里部分 `detail` 字段的搜索日志有截断，属于标准输出与标准错误到达顺序导致，落子与计时记录完整。
