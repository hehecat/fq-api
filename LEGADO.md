# 阅读 / Legado 书源

适配 [legado-Sigma-2026](https://github.com/angel888k/legado-Sigma-2026) 与原版阅读。

## 推荐：Full 完整正文

文件：[`legado-booksource-full.json`](./legado-booksource-full.json)  
后端：https://fq-full.oyufen.com  

| 规则 | 字段 |
|---|---|
| 搜索列表 | `$.data.books` |
| 书名/作者 | `$.book_name` / `$.author` |
| 书链接 | `$.book_url`（绝对 URL） |
| 目录 | `$.data.toc_url` → `$.data.chapters` |
| 章节链接 | `$.url`（绝对 URL） |
| 正文 | `$.data.content`（HTML 段落） |

### 导入

1. 删除旧的「番茄FQ·Worker / Full」源（如有）
2. 本地导入 JSON，或把文件拷到手机后本地导入
3. 手机浏览器确认：https://fq-full.oyufen.com/health  
4. 搜索调试关键字：`我不是戏神`  
5. 打开较后章节（如第 30 章）应是完整正文，不是试读

## 备选：Worker 网页源

文件：[`legado-booksource.json`](./legado-booksource.json)  
后端：https://fq-api.hehecat.workers.dev  

- 搜索/详情/目录一般可用  
- **正文可能被网页试读截断**  
- `size` 必须 ≤ 10  

## 故障排查

| 现象 | 处理 |
|---|---|
| 搜索 参数有误 / 502 | `size>10`；用最新书源 `size=10` |
| 目录/正文失败 | 重导 Full 源；确认 `/health` 为 `fq-full-adapter` |
| 正文很短 | 你在用 Worker 网页源 → 换 Full 源 |
| health 打不开 | 网络/DNS；检查 VPS tunnel 与 adapter 服务 |
