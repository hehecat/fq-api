# 阅读 / Legado Sigma 书源

适配 [legado-Sigma-2026](https://github.com/angel888k/legado-Sigma-2026)（与原版 [gedoor/legado](https://github.com/gedoor/legado) 书源格式兼容）。

后端 Worker：`https://fq-api.hehecat.workers.dev`

## 一键导入（推荐）

手机已装「阅读」时，用浏览器打开（把域名换成你的 raw 地址）：

```text
legado://import/bookSource?src=https://raw.githubusercontent.com/hehecat/fq-api/main/legado-booksource.json
```

若 raw 尚未挂到 GitHub，用本地文件导入（见下）。

## 本地导入

1. 把 `legado-booksource.json` 传到手机  
2. 阅读 → **我的** → **书源** → 右上角 **☰ / +** → **本地导入**  
3. 选中该 JSON  
4. 搜索试一下：`我不是戏神`

也可在电脑开阅读 Web 服务后：

```bash
curl -X POST 'http://手机IP:1234/saveBookSources' \
  -H 'Content-Type: application/json' \
  --data-binary @legado-booksource.json
```

## 字段映射

| 阅读规则 | Worker 接口 |
|---|---|
| 搜索 | `GET /api/search?q=&page=&size=` |
| 详情 | `GET /api/book/{book_id}` |
| 目录 | `GET /api/book/{book_id}/directory` |
| 正文 | `GET /api/book/{book_id}/chapter/{item_id}` |

- 搜索列表：`$.data.books[*]` → `book_name` / `author` / `abstract_text` …  
- 目录：`$.data.chapters[*]` → `title` / `item_id`  
- 正文：`$.data.content`（Worker 已字体解码，纯文本）

## 分页说明

阅读里 `{{page}}` 从 **1** 开始；Worker API 的 `page` 是 **0 基**。  
书源 `searchUrl` 里用 `@js` 做了 `page-1` 转换。

## 调试建议

1. 书源管理 → 选中本源 → **调试**  
2. 搜索关键字用：`我不是戏神`  
3. 看日志是否 200，列表名是否正常汉字  
4. 点进目录 / 正文，确认 `source` 含 `web_ssr+font` 或 content 无乱码  

## 自定义 Worker 域名

若你部署了自己的 Worker，全局替换 JSON 里的：

```text
https://fq-api.hehecat.workers.dev
```

为你的域名即可。

## 限制

- 无「发现」分类（`enabledExplore=false`）  
- 正文走官网阅读页 + 字体表，不是 App 签名 full 接口  
- 大批量/高并发可能被上游限流；阅读里可把本书源并发调低  
- 字体 ID 变更后需更新 Worker 内映射表并重新 deploy  

## 相关

- Worker 源码：本目录 `src/`  
- Rust 完整版：https://github.com/hehecat/fq-api  
- 阅读 Sigma：https://github.com/angel888k/legado-Sigma-2026  
